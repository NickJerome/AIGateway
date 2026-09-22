//! Queue, retention and batching limits.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub queue_rows: usize,
    pub buffer_bytes: usize,
    pub batch_rows: usize,
    pub flush_ms: u64,
    pub body: bool,
    pub retain_rows: usize,
    pub retain_days: u64,
    pub retain_bytes: u64,
    pub usage_days: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            queue_rows: 4096,
            buffer_bytes: 16 << 20,
            batch_rows: 64,
            flush_ms: 200,
            body: true,
            retain_rows: 100000,
            retain_days: 30,
            retain_bytes: 5 << 30,
            usage_days: 90,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        if self.queue_rows == 0
            || self.buffer_bytes < 65536
            || self.buffer_bytes > u32::MAX as usize
            || self.batch_rows == 0
            || self.flush_ms == 0
            || self.retain_rows == 0
            || self.retain_days == 0
            || self.retain_bytes == 0
            || self.usage_days == 0
        {
            bail!("invalid telemetry limits");
        }
        Ok(())
    }
}
