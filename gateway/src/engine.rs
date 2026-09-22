use crate::limits::Limits;
use crate::runtime::clock::SliceClock;
use crate::runtime::executor::ExecutorHandle;
use crate::runtime::request::{RequestHandle, RequestTask};
use crate::worker::RequestPermit;
use anyhow::{Result, anyhow};
use rquickjs::{Class, Context, Ctx, FromJs, Function, Module, Object, Promise, Runtime, Value};
use std::cell::Cell;
use std::rc::Rc;

pub struct Invocation {
    pub url: String,
    pub method: String,
}

/// One request owns one Runtime and Context. It stays on its worker thread.
/// ```compile_fail
/// fn require_send<T: Send>() {}
/// require_send::<gateway::engine::Engine>();
/// ```
/// `invoke` consumes the Engine; an executed Engine cannot serve another request.
/// ```compile_fail
/// use gateway::{engine::Invocation, worker::Worker};
/// let engine = Worker::new().start_request().unwrap();
/// let code = gateway::bytecode::compile("plugin.mjs", "export default {fetch(){return 'ok'}}").unwrap().code;
/// let _ = engine.invoke(&code, Invocation { url: "/".into(), method: "GET".into() });
/// let _ = engine.invoke(&code, Invocation { url: "/".into(), method: "GET".into() });
/// ```
pub struct Engine {
    // Fields drop in declaration order: Context, Runtime, then admission permit.
    context: Context,
    runtime: Runtime,
    clock: Rc<SliceClock>,
    interrupted: Rc<Cell<bool>>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    limits: Limits,
    _permit: Option<RequestPermit>,
}

impl Engine {
    pub(crate) fn new(limits: Limits, permit: RequestPermit) -> Result<Self> {
        Self::create(limits, Some(permit))
    }
    pub(crate) fn durable(limits: Limits) -> Result<Self> {
        Self::create(limits, None)
    }
    fn create(limits: Limits, permit: Option<RequestPermit>) -> Result<Self> {
        let runtime = Runtime::new()?;
        runtime.set_memory_limit(limits.memory_bytes);
        runtime.set_max_stack_size(limits.max_stack_bytes);
        let clock = Rc::new(SliceClock::new(limits.slice_ms));
        let interrupted = Rc::new(Cell::new(false));
        let watch = Rc::clone(&clock);
        let flag = Rc::clone(&interrupted);
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancelled = cancel.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            let expired = watch.expired()
                || watch.deadline_expired()
                || cancelled.load(std::sync::atomic::Ordering::SeqCst);
            if expired {
                flag.set(true);
            }
            expired
        })));
        let context = Context::full(&runtime)?;
        // Foundation Web API objects: installed once here so both entry paths
        // (the sync `invoke` adapter and the async request task) share it.
        context.with(|ctx| crate::web::install(&ctx))?;
        Ok(Self {
            context,
            runtime,
            clock,
            interrupted,
            cancel,
            limits,
            _permit: permit,
        })
    }

    pub(crate) fn cancellation(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.cancel.clone()
    }

    pub(crate) fn context(&self) -> &Context {
        &self.context
    }

    pub(crate) fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    pub(crate) fn clock(&self) -> &Rc<SliceClock> {
        &self.clock
    }

    pub(crate) fn interrupted(&self) -> bool {
        self.interrupted.get()
    }

    pub(crate) fn limits(&self) -> Limits {
        self.limits
    }

    /// Async entry point: the returned request task owns the Engine and its
    /// permit until Closed; the handle is the Send side for the HTTP layer.
    pub fn into_task(
        self,
        exec: &ExecutorHandle,
        code: &crate::bytecode::CompiledArtifact,
        call: Invocation,
    ) -> (RequestHandle, RequestTask) {
        RequestTask::new(self, exec.clone(), code, call)
    }

    pub fn invoke(
        self,
        code: &crate::bytecode::CompiledArtifact,
        call: Invocation,
    ) -> Result<String> {
        self.clock.start_segment();
        let result = self.context.with(|ctx| -> Result<String> {
            let exec = Object::new(ctx.clone())?;
            let returned = call_entry(&ctx, code, &call, exec)?;
            resolve(&ctx, returned)
        });
        // A failed request is abandoned; do not execute more plugin code to clean it.
        // Successful cleanup jobs keep the same slice, including microtask chains.
        let result = result.and_then(|body| self.drain_jobs().map(|_| body));
        if self.interrupted() {
            return Err(anyhow!("JavaScript execution slice exceeded"));
        }
        result
        // self drops before returning: Context -> Runtime -> permit.
    }

    fn drain_jobs(&self) -> Result<()> {
        while self.runtime.is_job_pending() {
            self.runtime
                .execute_pending_job()
                .map_err(|e| anyhow!("pending job failed: {e:?}"))?;
            if self.interrupted() {
                return Err(anyhow!("JavaScript execution slice exceeded"));
            }
        }
        Ok(())
    }
}

/// Shared module/entry invocation for the synchronous adapter and request task.
pub(crate) fn call_entry<'js>(
    ctx: &Ctx<'js>,
    code: &crate::bytecode::CompiledArtifact,
    call: &Invocation,
    exec: Object<'js>,
) -> Result<Value<'js>> {
    // Only locally compiled, build-validated bytecode reaches this API.
    crate::bytecode::declare_builtin(ctx)?;
    let declared = unsafe { Module::load(ctx.clone(), code) }
        .map_err(|e| describe(ctx, "load bytecode", e))?;
    let (module, initialized) = declared
        .eval()
        .map_err(|e| describe(ctx, "evaluate module", e))?;
    initialized
        .finish::<()>()
        .map_err(|e| describe(ctx, "initialize module", e))?;
    if call.method == "__DO" {
        let invoke: Function = ctx.globals().get("__gatewayDurableInvoke")?;
        return invoke
            .call((module.namespace()?, exec))
            .map_err(|e| describe(ctx, "DO entry", e));
    }
    let default: Object = module
        .get("default")
        .map_err(|_| anyhow!("plugin has no default export"))?;
    if call.method == "__SCHEDULED" {
        let handler: Function = default
            .get("scheduled")
            .map_err(|_| anyhow!("plugin has no scheduled entry"))?;
        let invoke: Function = ctx.globals().get("__gatewayScheduledInvoke")?;
        return invoke
            .call((handler, call.url.clone(), exec))
            .map_err(|e| describe(ctx, "scheduled()", e));
    }
    let fetch: Function = default
        .get("fetch")
        .map_err(|_| anyhow!("plugin default export has no fetch()"))?;
    if let Some(invoke) = ctx
        .globals()
        .get::<_, Option<Function>>("__gatewayInvoke")?
    {
        return invoke
            .call((fetch, call.url.clone(), call.method.clone(), exec))
            .map_err(|e| describe(ctx, "fetch()", e));
    }
    let request =
        crate::web::request::Request::incoming(ctx, call.url.clone(), call.method.clone())?;
    let env = Object::new(ctx.clone())?;
    let returned: Value = fetch
        .call((request, env, exec))
        .map_err(|e| describe(ctx, "fetch()", e))?;
    Ok(returned)
}

fn resolve<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<String> {
    if let Some(promise) = value.as_promise() {
        let promise: Promise<'js> = promise.clone();
        let settled: Value<'js> = promise
            .finish()
            .map_err(|e| describe(ctx, "await fetch()", e))?;
        return stringify(ctx, settled);
    }
    stringify(ctx, value)
}

pub(crate) fn stringify<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<String> {
    // A Response instance is consumed into its body text (spec 01: fetch may
    // return a Response; the sync adapter surface stays String-based).
    if let Ok(response) = Class::<crate::web::response::Response>::from_js(ctx, value.clone()) {
        return Ok(response.borrow().take_body_text());
    }
    value
        .try_into_string()
        .map_err(|_| anyhow!("fetch() must return a string or a Response"))?
        .to_string()
        .map_err(|e| describe(ctx, "read returned string", e))
}

pub(crate) fn describe(ctx: &Ctx<'_>, what: &str, err: rquickjs::Error) -> anyhow::Error {
    if matches!(err, rquickjs::Error::Exception) {
        let caught = ctx.catch();
        let text = caught
            .as_exception()
            .and_then(|e| e.message())
            .or_else(|| {
                caught
                    .as_object()
                    .and_then(|o| o.get::<_, String>("message").ok())
            })
            .unwrap_or_else(|| "unknown exception".to_string());
        anyhow!("{what}: {text}")
    } else {
        anyhow!("{what}: {err}")
    }
}
