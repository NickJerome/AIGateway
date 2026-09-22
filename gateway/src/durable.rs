//! Single-process object ownership. Only cloned values cross worker boundaries.
use crate::{plugins::Plugin, storage::Database};
use anyhow::{Result, anyhow, bail};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

pub fn object_id(namespace: &str, name: &str) -> String {
    crate::bytecode::source_sha256(&format!("{}:{namespace}{name}", namespace.len()))
}
pub fn owner(id: &str, threads: usize) -> usize {
    u64::from_str_radix(&id[..16], 16).expect("validated object ID") as usize % threads
}
pub struct Start {
    pub plugin: Arc<Plugin>,
    pub service: Arc<Service>,
}
struct Waiting {
    _bytes: crate::network::Reservation,
    object: String,
    reply: tokio::sync::oneshot::Sender<Result<Value>>,
}
pub struct Service {
    budget: crate::network::Budget,
    queue: async_channel::Sender<Value>,
    incoming: async_channel::Receiver<Value>,
    pending: Mutex<HashMap<u64, Waiting>>,
    next: AtomicU64,
    closed: AtomicBool,
}
impl Service {
    fn new(host_bytes: usize) -> Self {
        let (queue, incoming) = async_channel::bounded(1024);
        Self {
            budget: crate::network::Budget::new(host_bytes),
            queue,
            incoming,
            pending: Mutex::new(HashMap::new()),
            next: AtomicU64::new(0),
            closed: AtomicBool::new(false),
        }
    }
    pub fn budget(&self) -> crate::network::Budget {
        self.budget.clone()
    }
    pub async fn receive(&self) -> Result<Value> {
        if self.closed.load(Ordering::SeqCst) {
            bail!("DO service closed");
        }
        let value = self
            .incoming
            .recv()
            .await
            .map_err(|_| anyhow!("DO service closed"))?;
        if self.closed.load(Ordering::SeqCst) {
            bail!("DO service closed");
        }
        Ok(value)
    }
    pub fn reply(&self, id: u64, result: Result<Value>) {
        if let Some(waiting) = self.pending.lock().unwrap().remove(&id) {
            let _ = waiting.reply.send(result);
        }
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.queue.close();
        for (_, waiting) in self.pending.lock().unwrap().drain() {
            let _ = waiting.reply.send(Err(anyhow!("DO Engine closed")));
        }
    }
    async fn call(&self, mut message: Value) -> Result<Value> {
        let object = message["object"]
            .as_str()
            .ok_or_else(|| anyhow!("missing object ID"))?
            .to_owned();
        let bytes = self
            .budget
            .reserve(serde_json::to_vec(&message)?.len().saturating_mul(4))?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        {
            let mut pending = self.pending.lock().unwrap();
            if self.closed.load(Ordering::SeqCst) {
                bail!("DO Engine closed");
            }
            if pending.values().filter(|p| p.object == object).count() >= 128 {
                bail!("DO RPC queue full");
            }
            pending.insert(
                id,
                Waiting {
                    object,
                    reply: tx,
                    _bytes: bytes,
                },
            );
        }
        message["call"] = json!(id);
        if self.queue.try_send(message).is_err() {
            self.reply(id, Err(anyhow!("DO worker queue full")));
        }
        rx.await.map_err(|_| anyhow!("DO reply channel closed"))?
    }
}
pub struct Router {
    host_bytes: usize,
    workers: Vec<async_channel::Sender<Start>>,
    services: Mutex<HashMap<(u64, usize), Weak<Service>>>,
}
impl Router {
    pub fn new(
        threads: usize,
        host_bytes: usize,
    ) -> (Arc<Self>, Vec<async_channel::Receiver<Start>>) {
        let (workers, receivers) = (0..threads).map(|_| async_channel::bounded(128)).unzip();
        (
            Arc::new(Self {
                host_bytes,
                workers,
                services: Mutex::new(HashMap::new()),
            }),
            receivers,
        )
    }
    pub async fn call(&self, plugin: Arc<Plugin>, mut args: Value) -> Result<Value> {
        let binding = args["binding"]
            .as_str()
            .ok_or_else(|| anyhow!("missing DO binding"))?;
        let class = plugin
            .durable
            .get(binding)
            .ok_or_else(|| anyhow!("unknown DO binding"))?
            .clone();
        let name = args["name"]
            .as_str()
            .ok_or_else(|| anyhow!("invalid DO name"))?;
        if name.len() > 4096 {
            bail!("DO name exceeds 4096 bytes");
        }
        let id = object_id(&format!("{}:{class}", plugin.name), name);
        let worker = owner(&id, self.workers.len());
        let service = {
            let mut services = self.services.lock().unwrap();
            services.retain(|_, v| v.strong_count() > 0);
            let key = (plugin.generation, worker);
            if let Some(service) = services
                .get(&key)
                .and_then(Weak::upgrade)
                .filter(|s| !s.closed.load(Ordering::SeqCst))
            {
                service
            } else {
                let service = Arc::new(Service::new(self.host_bytes));
                self.workers[worker]
                    .try_send(Start {
                        plugin: plugin.clone(),
                        service: service.clone(),
                    })
                    .map_err(|_| anyhow!("DO owner unavailable"))?;
                services.insert(key, Arc::downgrade(&service));
                service
            }
        };
        args["object"] = json!(id);
        args["class"] = json!(class);
        service.call(args).await
    }
    pub fn close(&self) {
        for worker in &self.workers {
            worker.close();
        }
        for service in self
            .services
            .lock()
            .unwrap()
            .values()
            .filter_map(Weak::upgrade)
        {
            service.close();
        }
    }
}

impl Database {
    pub async fn migrate_durable(
        &self,
        plugin: String,
        migrations: Vec<crate::plugins::Migration>,
    ) -> Result<()> {
        self.run(move |conn| {
            let tx=conn.transaction()?;
            let old:Vec<(String,String)>=tx.prepare("SELECT tag,classes FROM do_migrations WHERE plugin=?1 ORDER BY position")?.query_map([&plugin],|r|Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
            if migrations.len()<old.len() {bail!("DO migration history cannot be removed");}
            for (i,m) in migrations.iter().enumerate() {
                let classes=serde_json::to_string(&m.new_sqlite_classes)?;
                if let Some((tag, stored))=old.get(i) {
                    if tag!=&m.tag || stored!=&classes {bail!("DO migration history changed");}
                } else {
                    tx.execute("INSERT INTO do_migrations(plugin,tag,position,classes) VALUES(?1,?2,?3,?4)",params![plugin,m.tag,i as i64,classes])?;
                    for class in &m.new_sqlite_classes {
                        let id=format!("do:{}:{class}",plugin);
                        tx.execute("INSERT INTO resources(id,kind,physical_name) VALUES(?1,'do',?2)",params![id,format!("do_{}",crate::bytecode::source_sha256(&id))])?;
                    }
                }
            }
            tx.commit()?;Ok(())
        }).await
    }
    pub async fn do_storage(
        &self,
        plugin: String,
        object: String,
        op: String,
        key: String,
        value: Option<String>,
    ) -> Result<Value> {
        if key.is_empty() || key.len() > 2048 {
            bail!("DO key must contain 1–2048 bytes");
        }
        if value.as_ref().is_some_and(|v| v.len() > 128 << 10) {
            bail!("DO value exceeds 128 KiB");
        }
        self.run(move |c| {
            match op.as_str() {
                "get" => Ok(json!(c.query_row("SELECT value FROM do_values WHERE plugin=?1 AND object=?2 AND key=?3",params![plugin,object,key],|r|r.get::<_,String>(0)).optional()?)),
                "put"=>{c.execute("INSERT INTO do_values VALUES(?1,?2,?3,?4) ON CONFLICT(plugin,object,key) DO UPDATE SET value=excluded.value",params![plugin,object,key,value.ok_or_else(||anyhow!("missing value"))?])?;Ok(Value::Null)},
                "delete"=>Ok(json!(c.execute("DELETE FROM do_values WHERE plugin=?1 AND object=?2 AND key=?3",params![plugin,object,key])?>0)),
                _=>bail!("unsupported DO storage operation")
            }
        }).await
    }
}
