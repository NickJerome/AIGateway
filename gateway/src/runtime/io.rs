//! Request-local promise roots; host futures carry owned bytes and data only.
use super::request::RequestHandle;
use crate::{
    network::{Budget, Chunk, FetchRequest, NativeBody, Network},
    storage::Namespace,
};
use anyhow::{Result, anyhow, bail};
use rquickjs::{ArrayBuffer, Ctx, Function, Object, Persistent, Promise, Value, function::Opt};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap},
    rc::Rc,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct DurableIo {
    pub router: Arc<crate::durable::Router>,
    pub plugin: Arc<crate::plugins::Plugin>,
    pub service: Option<Arc<crate::durable::Service>>,
}
pub struct IoConfig {
    pub identity: Option<crate::identity::Identity>,
    pub websocket: crate::websocket::Config,
    pub ws_capacity: Arc<tokio::sync::Semaphore>,
    pub upgrade: bool,
    pub durable: Option<DurableIo>,
    pub telemetry: Option<crate::telemetry::Telemetry>,
    pub request_id: String,
    pub trace: Option<crate::telemetry::Trace>,
    pub network: Network,
    pub runtime: tokio::runtime::Handle,
    pub vars: BTreeMap<String, serde_json::Value>,
    pub bindings: BTreeMap<String, Namespace>,
    pub headers: Vec<(String, String)>,
    pub body: Option<bytes::Bytes>,
}
pub(crate) enum Output {
    Json(serde_json::Value),
    Bytes(Chunk),
    Empty,
    WebSocket(crate::websocket::Event),
}
struct Pending {
    resolve: Persistent<Function<'static>>,
    reject: Persistent<Function<'static>>,
    rx: async_channel::Receiver<Result<Output>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.task.abort();
    }
}
#[derive(Default)]
struct Bodies {
    sockets: HashMap<u64, Arc<crate::websocket::Socket>>,
    next: u64,
    known: HashMap<u64, std::sync::Weak<NativeBody>>,
    values: HashMap<u64, Arc<NativeBody>>,
    uploads: HashMap<u64, async_channel::Sender<Result<Chunk>>>,
}
impl Bodies {
    fn socket(&mut self, s: Arc<crate::websocket::Socket>) -> u64 {
        self.next += 1;
        self.sockets.insert(self.next, s);
        self.next
    }
    fn add(&mut self, body: Arc<NativeBody>) -> u64 {
        self.next += 1;
        self.known.insert(self.next, Arc::downgrade(&body));
        self.values.insert(self.next, body);
        self.next
    }
}
pub(crate) struct Io {
    config: IoConfig,
    incoming: RefCell<Option<bytes::Bytes>>,
    handle: RequestHandle,
    bodies: Arc<Mutex<Bodies>>,
    budget: Budget,
    pending: RefCell<HashMap<u64, Pending>>,
    next: Cell<u64>,
    documents: RefCell<HashMap<u64, crate::ir::Document>>,
}
impl Io {
    pub fn new(mut config: IoConfig, handle: RequestHandle) -> Rc<Self> {
        let budget = config
            .durable
            .as_ref()
            .and_then(|d| d.service.as_ref())
            .map_or_else(
                || Budget::new(config.network.limits.host_buffer_bytes),
                |s| s.budget(),
            );
        let incoming = RefCell::new(config.body.take());
        Rc::new(Self {
            config,
            incoming,
            handle,
            bodies: Arc::new(Mutex::new(Bodies::default())),
            budget,
            pending: RefCell::new(HashMap::new()),
            next: Cell::new(0),
            documents: RefCell::new(HashMap::new()),
        })
    }
    pub fn budget(&self) -> Budget {
        self.budget.clone()
    }
    pub fn pending(&self) -> bool {
        !self.pending.borrow().is_empty()
    }
    pub fn socket(&self, id: u64) -> Result<Arc<crate::websocket::Socket>> {
        let socket = self
            .bodies
            .lock()
            .unwrap()
            .sockets
            .remove(&id)
            .ok_or_else(|| anyhow!("unknown WebSocket"))?;
        if socket.accepted.load(std::sync::atomic::Ordering::SeqCst) {
            bail!("cannot return an accepted WebSocket");
        }
        socket
            .detached
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(socket)
    }
    pub fn body(&self, id: u64) -> Result<Arc<NativeBody>> {
        self.bodies
            .lock()
            .unwrap()
            .values
            .remove(&id)
            .ok_or_else(|| anyhow!("body has already been consumed"))
    }
    pub fn abort_signal(&self, ctx: &Ctx<'_>) {
        if let Ok(Some(abort)) = ctx.globals().get::<_, Option<Function>>("__gatewayAbort")
            && abort.call::<_, ()>(()).is_err()
        {
            let _ = ctx.catch();
        }
    }
    pub fn clear(&self) {
        if let Some(service) = self
            .config
            .durable
            .as_ref()
            .and_then(|d| d.service.as_ref())
        {
            service.close();
        }
        self.pending.borrow_mut().clear();
        self.documents.borrow_mut().clear();
        let mut bodies = self.bodies.lock().unwrap();
        for body in bodies.known.values().filter_map(|w| w.upgrade()) {
            body.cancel();
        }
        for socket in bodies.sockets.values() {
            if !socket.detached.load(std::sync::atomic::Ordering::SeqCst) {
                socket.cancel();
            }
        }
        bodies.sockets.clear();
        bodies.known.clear();
        bodies.uploads.clear();
        bodies.values.clear();
    }
    pub fn install<'js>(self: &Rc<Self>, ctx: &Ctx<'js>) -> Result<()> {
        let incoming = if let Some(body) = self.incoming.borrow_mut().take() {
            Some(
                self.bodies
                    .lock()
                    .unwrap()
                    .add(NativeBody::bytes(body.clone(), self.budget.clone())?),
            )
        } else {
            None
        };
        let durable = self
            .config
            .durable
            .as_ref()
            .map(|d| {
                d.plugin
                    .durable
                    .iter()
                    .map(|(b, c)| (b.clone(), format!("{}:{c}", d.plugin.name)))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        let boot = serde_json::json!({"identity":self.config.identity,"upgrade":self.config.upgrade,"unsafe":self.config.durable.as_ref().map(|d|d.plugin.unsafe_bindings.clone()).unwrap_or_default(),"durable":durable,"headers":self.config.headers,"body":incoming,"vars":self.config.vars,"bindings":self.config.bindings.keys().collect::<Vec<_>>()});
        ctx.globals()
            .set("__gatewayBoot", ctx.json_parse(boot.to_string())?)?;
        let io = Rc::clone(self);
        ctx.globals().set(
            "__nativeBody",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>, value: Opt<Value<'js>>| -> rquickjs::Result<u64> {
                    let result = (|| -> Result<u64> {
                        if let Some(value) = value.0 {
                            let bytes = crate::web::body::bytes_argument(&ctx, value)?;
                            Ok(io
                                .bodies
                                .lock()
                                .unwrap()
                                .add(NativeBody::bytes(bytes.into(), io.budget.clone())?))
                        } else {
                            let (body, tx) = NativeBody::upload(
                                io.budget.clone(),
                                io.config.network.limits.queue_bytes,
                            );
                            let mut bodies = io.bodies.lock().unwrap();
                            let id = bodies.add(body);
                            bodies.uploads.insert(id, tx);
                            Ok(id)
                        }
                    })();
                    result.map_err(|e| rquickjs::Exception::throw_type(&ctx, &e.to_string()))
                },
            )?,
        )?;
        let io = Rc::clone(self);
        ctx.globals().set(
            "__cloneBody",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>, id: u64| -> rquickjs::Result<Option<u64>> {
                    let result = (|| -> Result<Option<u64>> {
                        let mut bodies = io.bodies.lock().unwrap();
                        let body = bodies
                            .values
                            .get(&id)
                            .ok_or_else(|| anyhow!("body consumed"))?
                            .fork()?;
                        Ok(body.map(|b| bodies.add(b)))
                    })();
                    result.map_err(|e| rquickjs::Exception::throw_type(&ctx, &e.to_string()))
                },
            )?,
        )?;
        let io = Rc::clone(self);
        ctx.globals().set(
            "__host",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>, op: String, args: String, data: Opt<Value<'js>>| {
                    io.start(&ctx, op, args, data.0)
                },
            )?,
        )?;
        let io = Rc::clone(self);
        ctx.globals().set(
            "__cancelHost",
            Function::new(ctx.clone(), move |id: u64| {
                io.pending.borrow_mut().remove(&id);
            })?,
        )?;
        ctx.globals().set(
            "__isProxy",
            Function::new(ctx.clone(), |value: Value| unsafe {
                rquickjs::qjs::JS_IsProxy(value.as_raw())
            })?,
        )?;
        ctx.globals().set(
            "__objectId",
            Function::new(ctx.clone(), |namespace: String, name: String| {
                crate::durable::object_id(&namespace, &name)
            })?,
        )?;
        let io = Rc::clone(self);
        ctx.globals().set(
            "__usageRecord",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>, text: String| -> rquickjs::Result<()> {
                    (|| -> Result<()> {
                        let plugin = &io
                            .config
                            .durable
                            .as_ref()
                            .ok_or_else(|| anyhow!("plugin unavailable"))?
                            .plugin
                            .name;
                        io.config
                            .telemetry
                            .as_ref()
                            .ok_or_else(|| anyhow!("telemetry unavailable"))?
                            .record(plugin, &io.config.request_id, serde_json::from_str(&text)?)
                    })()
                    .map_err(|e| rquickjs::Exception::throw_type(&ctx, &e.to_string()))
                },
            )?,
        )?;
        let io = Rc::clone(self);
        ctx.globals().set(
            "__recordLog",
            Function::new(ctx.clone(), move |level: String, text: String| {
                if let (Some(t), Some(d)) = (&io.config.telemetry, &io.config.durable) {
                    t.log(&d.plugin.name, &io.config.request_id, &level, text);
                }
            })?,
        )?;
        let io = Rc::clone(self);
        ctx.globals().set(
            "__irCreate",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>, id: u64| -> rquickjs::Result<u64> {
                    (|| -> Result<u64> {
                        let body = io.body(id)?;
                        let raw = body
                            .replayable()
                            .ok_or_else(|| anyhow!("IR.from requires a buffered request"))?;
                        let doc = crate::ir::Document::parse_budget(raw, Some(&io.budget))?;
                        let id = io.next.get() + 1;
                        io.next.set(id);
                        io.documents.borrow_mut().insert(id, doc);
                        Ok(id)
                    })()
                    .map_err(|e| rquickjs::Exception::throw_type(&ctx, &e.to_string()))
                },
            )?,
        )?;
        let io = Rc::clone(self);
        ctx.globals().set(
            "__irAccess",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>,
                      id: u64,
                      op: String,
                      path: String,
                      data: Opt<String>|
                      -> rquickjs::Result<Value<'js>> {
                    let result = (|| -> Result<serde_json::Value> {
                        let path: Vec<String> = serde_json::from_str(&path)?;
                        let mut docs = io.documents.borrow_mut();
                        let doc = docs
                            .get_mut(&id)
                            .ok_or_else(|| anyhow!("IR document closed"))?;
                        match op.as_str() {
                            "read" => doc.read(&path),
                            "set" | "delete" => {
                                doc.edit(
                                    &path,
                                    if op == "delete" {
                                        None
                                    } else {
                                        Some(serde_json::from_str(
                                            data.0
                                                .as_deref()
                                                .ok_or_else(|| anyhow!("missing value"))?,
                                        )?)
                                    },
                                )?;
                                Ok(serde_json::Value::Null)
                            }
                            _ => bail!("unknown IR operation"),
                        }
                    })();
                    let value = result
                        .map_err(|e| rquickjs::Exception::throw_type(&ctx, &e.to_string()))?;
                    ctx.json_parse(value.to_string())
                },
            )?,
        )?;
        let io = Rc::clone(self);
        ctx.globals().set(
            "__irSnapshot",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>, id: u64| -> rquickjs::Result<u64> {
                    (|| -> Result<u64> {
                        let docs = io.documents.borrow();
                        let doc = docs.get(&id).ok_or_else(|| anyhow!("IR document closed"))?;
                        let pieces = doc.snapshot_charged(&io.budget)?;
                        Ok(io
                            .bodies
                            .lock()
                            .unwrap()
                            .add(NativeBody::segments(pieces, io.budget.clone())?))
                    })()
                    .map_err(|e| rquickjs::Exception::throw_type(&ctx, &e.to_string()))
                },
            )?,
        )?;
        let io = Rc::clone(self);
        ctx.globals().set(
            "__wsPair",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
                    let result = (|| -> Result<_> {
                        let permit = Arc::new(
                            io.config
                                .ws_capacity
                                .clone()
                                .try_acquire_owned()
                                .map_err(|_| anyhow!("WebSocket capacity exhausted"))?,
                        );
                        let (a, b) = crate::websocket::Socket::pair(
                            io.budget.clone(),
                            io.config.websocket.max_message_bytes,
                            4,
                            String::new(),
                        );
                        *a.capacity.lock().unwrap() = Some(permit.clone());
                        *b.capacity.lock().unwrap() = Some(permit);
                        if let Some(d) = &io.config.durable {
                            d.plugin.socket(&a)?;
                            d.plugin.socket(&b)?;
                        }
                        let mut bodies = io.bodies.lock().unwrap();
                        Ok(serde_json::json!([bodies.socket(a), bodies.socket(b)]))
                    })();
                    ctx.json_parse(
                        result
                            .map_err(|e| rquickjs::Exception::throw_type(&ctx, &e.to_string()))?
                            .to_string(),
                    )
                },
            )?,
        )?;
        let io = Rc::clone(self);
        ctx.globals().set("__wsAction",Function::new(ctx.clone(),move|ctx:Ctx<'js>,id:u64,op:String,args:String,data:Opt<Value<'js>>|->rquickjs::Result<Value<'js>>{
            let result=(||->Result<serde_json::Value>{let socket=io.bodies.lock().unwrap().sockets.get(&id).cloned().ok_or_else(||anyhow!("WebSocket closed"))?;let args:serde_json::Value=serde_json::from_str(&args)?;
                match op.as_str(){"accept"=>{socket.accepted.store(true,std::sync::atomic::Ordering::SeqCst);},"text"=>socket.send("message_text",bytes::Bytes::from(args["text"].as_str().unwrap_or("").to_owned()))?,"binary"=>socket.send("message_binary",crate::web::body::bytes_argument(&ctx,data.0.ok_or_else(||anyhow!("missing WebSocket data"))?)?.into())?,"close"=>socket.close(args["code"].as_u64().unwrap_or(1000) as u16,args["reason"].as_str().unwrap_or("").into())?,"closed"=>socket.js_closed(),"meta"=>{},_=>bail!("unknown WebSocket action")};Ok(serde_json::json!({"state":socket.state.load(std::sync::atomic::Ordering::SeqCst),"protocol":*socket.protocol.lock().unwrap(),"url":socket.url}))})();ctx.json_parse(result.map_err(|e|rquickjs::Exception::throw_type(&ctx,&e.to_string()))?.to_string())
        })?)?;
        ctx.globals().set(
            "__installWebSockets",
            Function::new(ctx.clone(), |ctx: Ctx<'js>| {
                ctx.eval::<(), _>(include_str!("../web/websocket.js"))
            })?,
        )?;
        ctx.eval::<(),_>("for(const name of ['WebSocket','WebSocketPair','MessageEvent','CloseEvent','ErrorEvent'])Object.defineProperty(globalThis,name,{configurable:true,get(){for(const key of ['WebSocket','WebSocketPair','MessageEvent','CloseEvent','ErrorEvent'])delete globalThis[key];__installWebSockets();return globalThis[name];}});")?;
        if self
            .config
            .durable
            .as_ref()
            .is_some_and(|d| !d.plugin.durable.is_empty())
        {
            ctx.eval::<(), _>(include_str!("../web/clone-wire.js"))?;
            ctx.eval::<(), _>(include_str!("../web/durable.js"))?;
        }
        if self
            .config
            .durable
            .as_ref()
            .is_some_and(|d| d.plugin.unsafe_bindings.values().any(|k| k == "ir"))
        {
            ctx.eval::<(), _>(include_str!("../web/ir.js"))?;
            ctx.eval::<(), _>(include_str!("../web/ir-fields.js"))?;
            ctx.eval::<(), _>(include_str!("../web/ir-response.js"))?;
        }
        ctx.eval::<(), _>(include_str!("../web/http.js"))?;
        Ok(())
    }
    fn start<'js>(
        self: &Rc<Self>,
        ctx: &Ctx<'js>,
        op: String,
        args: String,
        data: Option<Value<'js>>,
    ) -> rquickjs::Result<Object<'js>> {
        let (promise, resolve, reject) = Promise::new(ctx)?;
        let id = self.next.get() + 1;
        self.next.set(id);
        let (tx, rx) = async_channel::bounded(1);
        let io_bodies = self.bodies.clone();
        let budget = self.budget.clone();
        let network = self.config.network.clone();
        let bindings = self.config.bindings.clone();
        let durable = self.config.durable.clone();
        let telemetry = self.config.telemetry.clone();
        let trace = self.config.trace.clone();
        let websocket = self.config.websocket.clone();
        let ws_capacity = self.config.ws_capacity.clone();
        let handle = self.handle.clone();
        let parsed = (|| -> Result<_> {
            let args: serde_json::Value = serde_json::from_str(&args)?;
            let data = data
                .filter(|v| !v.is_undefined())
                .map(|v| crate::web::body::bytes_argument(ctx, v))
                .transpose()?
                .map(|v| Chunk::new(v.into(), &budget))
                .transpose()?;
            Ok((args, data))
        })();
        let task = self.config.runtime.spawn(async move {
            let result = match parsed {
                Ok((args, data)) => {
                    operation(
                        op,
                        args,
                        data,
                        io_bodies,
                        network,
                        bindings,
                        budget,
                        durable,
                        telemetry,
                        trace,
                        websocket,
                        ws_capacity,
                    )
                    .await
                }
                Err(e) => Err(e),
            };
            let _ = tx.send(result).await;
            handle.io_ready();
        });
        self.pending.borrow_mut().insert(
            id,
            Pending {
                resolve: Persistent::save(ctx, resolve),
                reject: Persistent::save(ctx, reject),
                rx,
                task,
            },
        );
        let out = Object::new(ctx.clone())?;
        out.set("id", id)?;
        out.set("promise", promise)?;
        Ok(out)
    }
    pub fn complete(&self, ctx: &Ctx<'_>) -> Result<()> {
        self.handle.clear_io_ready();
        let ids = self
            .pending
            .borrow()
            .iter()
            .filter(|(_, p)| !p.rx.is_empty() || p.rx.is_closed())
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in ids {
            let pending = self.pending.borrow_mut().remove(&id).unwrap();
            let result = pending
                .rx
                .try_recv()
                .unwrap_or_else(|_| Err(anyhow!("host operation stopped")));
            match result {
                Ok(value) => {
                    let resolve = pending.resolve.clone().restore(ctx)?;
                    match value {
                        Output::Json(value) => {
                            resolve.call::<_, ()>((ctx.json_parse(value.to_string())?,))?
                        }
                        Output::Bytes(chunk) => resolve.call::<_, ()>((ArrayBuffer::new_copy(
                            ctx.clone(),
                            chunk.bytes.as_ref(),
                        )?,))?,
                        Output::WebSocket(event) => {
                            let object = Object::new(ctx.clone())?;
                            object.set("kind", event.kind)?;
                            object.set("code", event.code)?;
                            object.set("reason", event.reason)?;
                            if let Some(data) = event.data {
                                if event.kind == "message_text" {
                                    object.set(
                                        "data",
                                        String::from_utf8_lossy(&data.bytes).into_owned(),
                                    )?;
                                } else {
                                    object.set(
                                        "data",
                                        ArrayBuffer::new_copy(ctx.clone(), data.bytes.as_ref())?,
                                    )?;
                                }
                            }
                            resolve.call::<_, ()>((object,))?;
                        }
                        Output::Empty => resolve.call::<_, ()>((Value::new_null(ctx.clone()),))?,
                    }
                }
                Err(error) => {
                    let reject = pending.reject.clone().restore(ctx)?;
                    reject.call::<_, ()>((crate::web::named_error(
                        ctx,
                        "Error",
                        &format!("{error:#}"),
                    )?,))?;
                }
            }
        }
        Ok(())
    }
}
#[allow(clippy::too_many_arguments)]
async fn operation(
    op: String,
    args: serde_json::Value,
    data: Option<Chunk>,
    bodies: Arc<Mutex<Bodies>>,
    network: Network,
    bindings: BTreeMap<String, Namespace>,
    budget: Budget,
    durable: Option<DurableIo>,
    telemetry: Option<crate::telemetry::Telemetry>,
    trace: Option<crate::telemetry::Trace>,
    websocket: crate::websocket::Config,
    ws_capacity: Arc<tokio::sync::Semaphore>,
) -> Result<Output> {
    let id = || {
        args["id"]
            .as_u64()
            .ok_or_else(|| anyhow!("missing body ID"))
    };
    if op == "ws.receive" {
        let socket = bodies
            .lock()
            .unwrap()
            .sockets
            .get(&id()?)
            .cloned()
            .ok_or_else(|| anyhow!("WebSocket closed"))?;
        return Ok(Output::WebSocket(socket.receive().await?));
    }
    if op == "ws.connect"
        || op == "fetch"
            && args["headers"].as_array().is_some_and(|hs| {
                hs.iter().any(|h| {
                    h[0].as_str()
                        .is_some_and(|k| k.eq_ignore_ascii_case("upgrade"))
                        && h[1]
                            .as_str()
                            .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
                })
            })
    {
        let permit = Arc::new(
            ws_capacity
                .try_acquire_owned()
                .map_err(|_| anyhow!("WebSocket capacity exhausted"))?,
        );
        let request: FetchRequest = serde_json::from_value(args)?;
        let target = request.url.clone();
        let proxy = request
            .proxy
            .split(':')
            .next()
            .unwrap_or("direct")
            .to_owned();
        let result = network.upgrade(request, budget, websocket).await;
        if let Some(trace) = trace {
            trace.metadata(serde_json::json!({"event":"attempt","target":target,"proxy":proxy,"status":result.as_ref().ok().map(|(r,_)|r.status),"error":result.as_ref().err().map(ToString::to_string),"websocket":true})).await?;
        }
        let (response, socket) = result?;
        if let (Some(d), Some(s)) = (&durable, &socket) {
            d.plugin.socket(s)?;
            *s.capacity.lock().unwrap() = Some(permit);
        }
        let mut bodies = bodies.lock().unwrap();
        let socket = socket.map(|s| bodies.socket(s));
        let body = bodies.add(response.body);
        return Ok(Output::Json(
            serde_json::json!({"status":response.status,"headers":response.headers,"url":response.url,"redirected":false,"body":body,"webSocket":socket}),
        ));
    }
    if op == "usage.query" {
        let plugin = durable
            .as_ref()
            .ok_or_else(|| anyhow!("plugin unavailable"))?
            .plugin
            .name
            .clone();
        let group: Vec<String> = serde_json::from_value(args["groupBy"].clone())?;
        return Ok(Output::Json(
            telemetry
                .ok_or_else(|| anyhow!("telemetry unavailable"))?
                .query(plugin, args["since"].as_i64().unwrap_or(0), group)
                .await?,
        ));
    }
    if op.starts_with("do.") {
        let d = durable.ok_or_else(|| anyhow!("DO service unavailable"))?;
        let service = || d.service.as_ref().ok_or_else(|| anyhow!("not a DO Engine"));
        let value = match op.as_str() {
            "do.call" => d.router.call(d.plugin, args).await?,
            "do.receive" => service()?.receive().await?,
            "do.reply" => {
                let result = if let Some(error) = args["error"].as_str() {
                    if error.to_ascii_lowercase().contains("out of memory") {
                        service()?.close();
                    }
                    Err(anyhow!("{error}"))
                } else {
                    Ok(args["value"].clone())
                };
                service()?.reply(
                    args["call"]
                        .as_u64()
                        .ok_or_else(|| anyhow!("missing call"))?,
                    result,
                );
                serde_json::Value::Null
            }
            "do.storage" => {
                service()?;
                d.plugin
                    .database
                    .do_storage(
                        d.plugin.name.clone(),
                        args["object"]
                            .as_str()
                            .ok_or_else(|| anyhow!("missing object"))?
                            .into(),
                        args["op"].as_str().unwrap_or("").into(),
                        args["key"].as_str().unwrap_or("").into(),
                        args["value"].as_str().map(str::to_owned),
                    )
                    .await?
            }
            _ => bail!("unknown DO operation"),
        };
        return Ok(Output::Json(value));
    }
    match op.as_str() {
        "bodyRead" => {
            let body = bodies
                .lock()
                .unwrap()
                .values
                .get(&id()?)
                .cloned()
                .ok_or_else(|| anyhow!("body is closed"))?;
            match body.read().await? {
                Some(chunk) => Ok(Output::Bytes(chunk)),
                None => {
                    bodies.lock().unwrap().values.remove(&id()?);
                    Ok(Output::Empty)
                }
            }
        }
        "bodyCancel" => {
            let id = id()?;
            let mut bodies = bodies.lock().unwrap();
            if let Some(body) = bodies.known.remove(&id).and_then(|w| w.upgrade()) {
                body.cancel();
            }
            bodies.values.remove(&id);
            bodies.uploads.remove(&id);
            Ok(Output::Empty)
        }
        "uploadWrite" => {
            let tx = bodies
                .lock()
                .unwrap()
                .uploads
                .get(&id()?)
                .cloned()
                .ok_or_else(|| anyhow!("upload is closed"))?;
            let data = data.ok_or_else(|| anyhow!("upload requires bytes"))?;
            if data.bytes.len() > 64 << 10 {
                bail!("upload chunk exceeds 64 KiB");
            }
            tx.send(Ok(data))
                .await
                .map_err(|_| anyhow!("upload cancelled"))?;
            Ok(Output::Empty)
        }
        "uploadClose" => {
            bodies.lock().unwrap().uploads.remove(&id()?);
            Ok(Output::Empty)
        }
        "uploadError" => {
            let tx = bodies.lock().unwrap().uploads.remove(&id()?);
            if let Some(tx) = tx {
                let _ = tx.send(Err(anyhow!("JS body stream failed"))).await;
            }
            Ok(Output::Empty)
        }
        "fetch" => {
            let body = match args["body"].as_u64() {
                Some(id) => Some(
                    bodies
                        .lock()
                        .unwrap()
                        .values
                        .remove(&id)
                        .ok_or_else(|| anyhow!("body consumed"))?,
                ),
                None => None,
            };
            let request: FetchRequest = serde_json::from_value(args)?;
            let target = request.url.clone();
            let proxy = request
                .proxy
                .split(':')
                .next()
                .unwrap_or("direct")
                .to_string();
            let result = network.fetch(request, body, budget).await;
            if let Some(trace) = trace {
                trace.metadata(serde_json::json!({"event":"attempt","target":target,"proxy":proxy,"status":result.as_ref().ok().map(|r|r.status),"error":result.as_ref().err().map(ToString::to_string)})).await?;
            }
            let r = result?;
            let id = bodies.lock().unwrap().add(r.body);
            Ok(Output::Json(
                serde_json::json!({"status":r.status,"headers":r.headers,"url":r.url,"redirected":r.redirected,"body":id}),
            ))
        }
        "kv.get" | "kv.put" | "kv.delete" | "kv.list" => {
            let name = args["binding"]
                .as_str()
                .ok_or_else(|| anyhow!("missing binding"))?;
            let kv = bindings
                .get(name)
                .ok_or_else(|| anyhow!("unknown KV binding"))?;
            let key = || {
                args["key"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| anyhow!("missing KV key"))
            };
            match op.as_str() {
                "kv.get" => match kv.get(key()?).await? {
                    Some(bytes) => Ok(Output::Bytes(Chunk::new(bytes.into(), &budget)?)),
                    None => Ok(Output::Empty),
                },
                "kv.put" => {
                    let data = data.ok_or_else(|| anyhow!("missing KV value"))?;
                    kv.put(key()?, data.bytes.clone(), args["ttl"].as_u64())
                        .await?;
                    Ok(Output::Empty)
                }
                "kv.delete" => {
                    kv.delete(key()?).await?;
                    Ok(Output::Empty)
                }
                _ => Ok(Output::Json(serde_json::to_value(
                    kv.list(
                        args["prefix"].as_str().unwrap_or("").into(),
                        args["limit"].as_u64().unwrap_or(1000) as usize,
                    )
                    .await?,
                )?)),
            }
        }
        _ => bail!("unsupported host operation"),
    }
}
