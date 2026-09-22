//! Per-request trace lifetime, byte accounting and cancellation.
use super::{Telemetry, now};
use anyhow::Result;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};
#[derive(Clone)]
pub struct Trace(Arc<TraceState>);
struct TraceState {
    telemetry: Telemetry,
    plugin: String,
    id: String,
    started: Instant,
    seq: [AtomicU64; 2],
    bytes: [AtomicU64; 2],
    hash: Mutex<[sha2::Sha256; 2]>,
    ended: AtomicBool,
    body: bool,
}
impl Trace {
    pub async fn begin(
        telemetry: Telemetry,
        plugin: String,
        method: String,
        url: String,
        headers: Vec<(String, String)>,
    ) -> Result<Self> {
        Self::begin_configured(telemetry, plugin, method, url, headers, None).await
    }
    pub async fn begin_configured(
        telemetry: Telemetry,
        plugin: String,
        method: String,
        url: String,
        headers: Vec<(String, String)>,
        body: Option<bool>,
    ) -> Result<Self> {
        use sha2::Digest;
        let body = body.unwrap_or(telemetry.body_enabled());
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = format!(
            "{:x}-{:x}-{:x}",
            std::process::id(),
            now(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        telemetry.trace(&plugin,&id,json!({"event":"start","request_id":id,"plugin_id":plugin,"method":method,"url":url,"request_headers":headers,"complete":false,"body_recorded":body}),None).await?;
        Ok(Self(Arc::new(TraceState {
            telemetry,
            plugin,
            id,
            started: Instant::now(),
            seq: std::array::from_fn(|_| AtomicU64::new(0)),
            bytes: std::array::from_fn(|_| AtomicU64::new(0)),
            hash: Mutex::new([sha2::Sha256::new(), sha2::Sha256::new()]),
            ended: AtomicBool::new(false),
            body,
        })))
    }
    pub fn id(&self) -> &str {
        &self.0.id
    }
    pub async fn chunk(&self, side: usize, bytes: bytes::Bytes) -> Result<()> {
        use sha2::Digest;
        self.0.bytes[side].fetch_add(bytes.len() as u64, Ordering::Relaxed);
        self.0.hash.lock().unwrap()[side].update(&bytes);
        if self.0.body {
            for part in bytes.chunks(64 << 10) {
                let seq = self.0.seq[side].fetch_add(1, Ordering::Relaxed);
                self.0.telemetry.trace(&self.0.plugin,&self.0.id,json!({"event":"chunk","side":if side==0{"request"}else{"response"},"seq":seq}),Some(bytes::Bytes::copy_from_slice(part))).await?;
            }
        }
        Ok(())
    }
    pub async fn metadata(&self, data: Value) -> Result<()> {
        self.0
            .telemetry
            .trace(&self.0.plugin, &self.0.id, data, None)
            .await
    }
    pub async fn finish(&self, complete: bool, error: Option<String>) -> Result<()> {
        use sha2::Digest;
        if self.0.ended.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let hashes = self
            .0
            .hash
            .lock()
            .unwrap()
            .clone()
            .map(|h| hex::encode(h.finalize()));
        self.metadata(json!({"event":"end","complete":complete,"error":error,"duration_ms":self.0.started.elapsed().as_millis() as u64,"request_bytes":self.0.bytes[0].load(Ordering::Relaxed),"response_bytes":self.0.bytes[1].load(Ordering::Relaxed),"request_hash":hashes[0],"response_hash":hashes[1]})).await
    }
    pub fn cancelled(&self) {
        if !self.0.ended.load(Ordering::SeqCst) {
            let trace = self.clone();
            tokio::spawn(async move {
                let _ = trace
                    .finish(false, Some("request cancelled or transport failed".into()))
                    .await;
            });
        }
    }
}
