use anyhow::{Result, anyhow};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: String,
    pub data_dir: PathBuf,
    pub threads: usize,
    pub limits: crate::limits::Limits,
    pub http: crate::network::HttpLimits,
    pub telemetry: crate::telemetry::Config,
    pub websocket: crate::websocket::Config,
    pub database_threads: usize,
    pub database_queue: usize,
    pub plugins_dir: PathBuf,
    pub admin_token_env: Option<String>,
    pub zip_upload_bytes: usize,
    pub zip_expanded_bytes: usize,
    pub zip_files: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    listen: String,
    data_dir: PathBuf,
    threads: Option<usize>,
    memory_limit_mb: Option<usize>,
    max_in_flight: Option<usize>,
    #[serde(default)]
    http: crate::network::HttpLimits,
    #[serde(default)]
    telemetry: crate::telemetry::Config,
    #[serde(default)]
    websocket: crate::websocket::Config,
    database_threads: Option<usize>,
    database_queue: Option<usize>,
    plugins_dir: Option<PathBuf>,
    admin_token_env: Option<String>,
    zip_upload_bytes: Option<usize>,
    zip_expanded_bytes: Option<usize>,
    zip_files: Option<usize>,
}

impl Config {
    pub fn from_toml(text: &str) -> Result<Config> {
        let raw: RawConfig = toml::from_str(text)?;
        raw.telemetry.validate()?;
        if raw.websocket.max_message_bytes == 0
            || raw.websocket.max_connections == 0
            || raw.websocket.close_timeout_ms == 0
        {
            return Err(anyhow!("invalid WebSocket capacity"));
        }
        let threads = match raw.threads {
            // An explicit zero is a configuration error; only an omitted
            // value falls back to the CPU quota derivation.
            Some(0) => {
                return Err(anyhow!(
                    "threads = 0 is not a valid worker count; omit threads to derive it"
                ));
            }
            Some(n) => n,
            None => threads_from_cpu_max(read_cpu_quota(), num_cores()),
        };
        let mut limits = crate::limits::Limits::default();
        if let Some(mb) = raw.memory_limit_mb {
            if mb == 0 {
                return Err(anyhow!(
                    "memory_limit_mb = 0 would admit unexecutable requests"
                ));
            }
            limits.memory_bytes = mb
                .checked_mul(1024 * 1024)
                .ok_or_else(|| anyhow!("memory_limit_mb = {mb} does not fit into usize bytes"))?;
        }
        if let Some(n) = raw.max_in_flight {
            if n == 0 {
                return Err(anyhow!("max_in_flight = 0 would reject every request"));
            }
            limits.max_in_flight = n;
        }
        let database_threads = raw.database_threads.unwrap_or(4);
        let database_queue = raw.database_queue.unwrap_or(128);
        let zip_upload_bytes = raw.zip_upload_bytes.unwrap_or(32 << 20);
        let zip_expanded_bytes = raw.zip_expanded_bytes.unwrap_or(128 << 20);
        let zip_files = raw.zip_files.unwrap_or(1024);
        if !(1..=4).contains(&database_threads)
            || database_queue == 0
            || zip_upload_bytes == 0
            || zip_expanded_bytes == 0
            || zip_files == 0
            || raw.http.queue_bytes < 64 << 10
            || raw.http.host_buffer_bytes == 0
            || raw.http.request_body_bytes == 0
            || raw.http.connect_timeout_ms == 0
            || raw.http.headers_timeout_ms == 0
            || raw.http.idle_timeout_ms == 0
            || raw.http.drain_timeout_ms == 0
        {
            return Err(anyhow!("invalid host capacity or timeout configuration"));
        }
        let plugins_dir = raw
            .plugins_dir
            .unwrap_or_else(|| raw.data_dir.join("plugins"));
        Ok(Config {
            http: raw.http,
            telemetry: raw.telemetry,
            websocket: raw.websocket,
            database_threads,
            database_queue,
            plugins_dir,
            admin_token_env: raw.admin_token_env,
            zip_upload_bytes,
            zip_expanded_bytes,
            zip_files,
            listen: raw.listen,
            data_dir: raw.data_dir,
            threads,
            limits,
        })
    }
}

pub fn threads_from_cpu_max(quota: Option<(u64, u64)>, cores: usize) -> usize {
    match quota {
        Some((q, p)) if p > 0 => std::cmp::max(1, (q / p) as usize),
        _ => std::cmp::max(1, cores),
    }
}

/// Effective cpu.max quota for the current process: the tightest finite
/// quota found at the process's cgroup v2 directory and every ancestor up to
/// `cgroup_root`. `proc_cgroup` is the content of `/proc/self/cgroup`;
/// injecting both parameters keeps this function testable with fixture trees
/// instead of touching the host cgroup hierarchy.
pub fn effective_cpu_quota(cgroup_root: &Path, proc_cgroup: &str) -> Option<(u64, u64)> {
    // cgroup v2 mounts appear as a single "0::<path>" line.
    let relative = proc_cgroup
        .lines()
        .find_map(|line| line.strip_prefix("0::"))?;
    let mut dir = cgroup_root.join(relative.trim_start_matches('/'));
    let mut best: Option<(u64, u64)> = None;
    loop {
        if let Some(quota) = read_cpu_max_file(&dir.join("cpu.max")) {
            best = Some(match best {
                Some(current) => tighter(current, quota),
                None => quota,
            });
        }
        if dir == cgroup_root || !dir.pop() {
            break;
        }
    }
    best
}

/// The tighter of two quotas, compared as quota/period ratios. Cross-
/// multiplication in u128 cannot overflow for u64 operands.
fn tighter(a: (u64, u64), b: (u64, u64)) -> (u64, u64) {
    if u128::from(a.0) * u128::from(b.1) <= u128::from(b.0) * u128::from(a.1) {
        a
    } else {
        b
    }
}

/// Parses one cpu.max file; "max" (unlimited) and unreadable files yield None.
fn read_cpu_max_file(path: &Path) -> Option<(u64, u64)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut parts = text.split_whitespace();
    let quota = parts.next()?;
    let period: u64 = parts.next()?.parse().ok()?;
    if quota == "max" {
        return None;
    }
    Some((quota.parse().ok()?, period))
}

fn read_cpu_quota() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    effective_cpu_quota(Path::new("/sys/fs/cgroup"), &text)
}

fn num_cores() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}
