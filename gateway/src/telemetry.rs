//! Telemetry facade: bounded admission, batching lifecycle and explicit record kinds.
use crate::storage::Database;
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
mod config;
mod query;
mod trace;
mod writer;
pub use config::Config;
pub use query::{Page, Record};
pub use trace::Trace;

/// Closed set of Core record kinds; plugin-defined fields belong in the payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Usage,
    Logs,
    Traces,
}
impl Kind {
    fn queue_index(self) -> usize {
        match self {
            Self::Usage => 0,
            Self::Logs => 1,
            Self::Traces => 2,
        }
    }
}
#[derive(Clone)]
pub struct Telemetry(Arc<Inner>);
struct Inner {
    tx: mpsc::Sender<Command>,
    rows: [Arc<Semaphore>; 3],
    bytes: Arc<Semaphore>,
    pub config: Config,
    db: Database,
    closed: AtomicBool,
    join: Mutex<Option<std::thread::JoinHandle<()>>>,
    pub dropped: AtomicU64,
    pub failed: Arc<AtomicU64>,
}
struct Row {
    kind: Kind,
    plugin: String,
    request: String,
    time: i64,
    data: Value,
    body: Option<bytes::Bytes>,
    _rows: OwnedSemaphorePermit,
    _bytes: OwnedSemaphorePermit,
}
enum Command {
    Row(Row),
    Flush(tokio::sync::oneshot::Sender<Result<()>>),
    Stop(tokio::sync::oneshot::Sender<Result<()>>, Instant),
}
impl Drop for Inner {
    fn drop(&mut self) {
        let (tx, _) = tokio::sync::oneshot::channel();
        let _ = self
            .tx
            .send(Command::Stop(tx, Instant::now() + Duration::from_secs(30)));
        if let Some(join) = self.join.get_mut().unwrap().take() {
            let _ = join.join();
        }
    }
}
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
pub enum ListCursor {
    After(i64),
    Before(i64),
}

impl Telemetry {
    pub fn open(dir: &Path, db: Database, config: Config) -> Result<Self> {
        config.validate()?;
        let conn = writer::open(dir)?;
        let (tx, rx) = mpsc::channel();
        let limits = config.clone();
        let failed = Arc::new(AtomicU64::new(0));
        let errors = failed.clone();
        let join = std::thread::Builder::new()
            .name("telemetry".into())
            .spawn(move || writer::run(conn, rx, limits, errors))?;
        Ok(Self(Arc::new(Inner {
            tx,
            rows: std::array::from_fn(|_| Arc::new(Semaphore::new(config.queue_rows))),
            bytes: Arc::new(Semaphore::new(config.buffer_bytes)),
            config,
            db,
            closed: AtomicBool::new(false),
            join: Mutex::new(Some(join)),
            dropped: AtomicU64::new(0),
            failed,
        })))
    }
    pub fn body_enabled(&self) -> bool {
        self.0.config.body
    }
    pub fn record(&self, plugin: &str, request: &str, value: Value) -> Result<()> {
        let object = value
            .as_object()
            .ok_or_else(|| anyhow!("usage record requires scalar fields"))?;
        if object.keys().any(|k| {
            matches!(
                k.as_str(),
                "plugin_id" | "request_id" | "time" | "timestamp"
            )
        }) || object.values().any(|v| {
            !matches!(
                v,
                Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null
            )
        }) {
            bail!("usage contains invalid or reserved fields");
        }
        if serde_json::to_vec(&value)?.len() > 65536 {
            bail!("usage record exceeds 64 KiB");
        }
        self.enqueue(Kind::Usage, plugin, request, value, None)
    }
    pub fn log(&self, plugin: &str, request: &str, level: &str, message: String) {
        if self
            .enqueue(
                Kind::Logs,
                plugin,
                request,
                json!({"level":level,"message":message}),
                None,
            )
            .is_err()
        {
            let dropped = self.0.dropped.fetch_add(1, Ordering::Relaxed) + 1;
            if dropped.is_power_of_two() {
                eprintln!("telemetry logs dropped={dropped}");
            }
        }
    }
    fn enqueue(
        &self,
        kind: Kind,
        plugin: &str,
        request: &str,
        data: Value,
        body: Option<bytes::Bytes>,
    ) -> Result<()> {
        if self.0.closed.load(Ordering::SeqCst) {
            bail!("telemetry closed");
        }
        let size = serde_json::to_vec(&data)?.len()
            + body.as_ref().map_or(0, bytes::Bytes::len)
            + plugin.len()
            + request.len()
            + 128;
        let rows = self.0.rows[kind.queue_index()]
            .clone()
            .try_acquire_owned()
            .map_err(|_| anyhow!("telemetry row queue full"))?;
        let bytes = self
            .0
            .bytes
            .clone()
            .try_acquire_many_owned(u32::try_from(size)?)
            .map_err(|_| anyhow!("telemetry byte queue full"))?;
        self.0
            .tx
            .send(Command::Row(Row {
                kind,
                plugin: plugin.into(),
                request: request.into(),
                time: now(),
                data,
                body,
                _rows: rows,
                _bytes: bytes,
            }))
            .map_err(|_| anyhow!("telemetry writer stopped"))
    }
    pub async fn trace(
        &self,
        plugin: &str,
        request: &str,
        data: Value,
        body: Option<bytes::Bytes>,
    ) -> Result<()> {
        if self.0.closed.load(Ordering::SeqCst) {
            bail!("telemetry closed");
        }
        let size = serde_json::to_vec(&data)?.len()
            + body.as_ref().map_or(0, bytes::Bytes::len)
            + plugin.len()
            + request.len()
            + 128;
        if size > self.0.config.buffer_bytes {
            bail!("trace chunk exceeds buffer limit");
        }
        let rows = self.0.rows[Kind::Traces.queue_index()]
            .clone()
            .acquire_owned()
            .await?;
        let bytes = self.0.bytes.clone().acquire_many_owned(size as u32).await?;
        self.0
            .tx
            .send(Command::Row(Row {
                kind: Kind::Traces,
                plugin: plugin.into(),
                request: request.into(),
                time: now(),
                data,
                body,
                _rows: rows,
                _bytes: bytes,
            }))
            .map_err(|_| anyhow!("telemetry stopped"))
    }
    pub async fn flush(&self) -> Result<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.0.tx.send(Command::Flush(tx))?;
        rx.await?
    }
    pub async fn shutdown(&self) -> Result<()> {
        self.shutdown_with_timeout(Duration::from_secs(30)).await
    }
    pub async fn shutdown_with_timeout(&self, timeout: Duration) -> Result<()> {
        self.0.closed.store(true, Ordering::SeqCst);
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.0
            .tx
            .send(Command::Stop(tx, Instant::now() + timeout))?;
        rx.await?
    }
    pub fn counters(&self) -> Value {
        json!({"buffer_bytes":self.0.config.buffer_bytes-self.0.bytes.available_permits(),"dropped_logs":self.0.dropped.load(Ordering::Relaxed),"failed_batches":self.0.failed.load(Ordering::Relaxed)})
    }
}
