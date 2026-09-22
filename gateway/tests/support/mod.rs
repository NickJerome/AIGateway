#![allow(dead_code)]
use std::io::Write;
pub fn zip_files(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
pub fn package(name: &str, source: &str, extra: &str) -> Vec<u8> {
    let manifest = format!("name='{name}'\nmain='dist/worker.mjs'\nroutes=['/{name}/*']\n{extra}");
    zip_files(&[
        ("wrangler.toml", manifest.as_bytes()),
        ("dist/worker.mjs", source.as_bytes()),
    ])
}
pub fn config(dir: &std::path::Path) -> gateway::config::Config {
    gateway::config::Config::from_toml(&format!(
        "listen='127.0.0.1:0'\ndata_dir='{}'\nthreads=1\n[http]\ndrain_timeout_ms=200\n",
        dir.display()
    ))
    .unwrap()
}
pub async fn install(dir: &std::path::Path, name: &str, source: &str, extra: &str) {
    let db = gateway::storage::Database::open(dir, 1, 8).unwrap();
    let mut manager =
        gateway::plugins::PluginManager::open(dir.join("plugins"), db, Default::default())
            .await
            .unwrap();
    manager.install(package(name, source, extra)).await.unwrap();
}

pub struct Process {
    pub child: std::process::Child,
    pub base: String,
}
impl Process {
    pub fn start(dir: &std::path::Path, extra: &str) -> Self {
        use std::io::BufRead;
        let path = dir.join("gateway.toml");
        std::fs::write(&path,format!("listen='127.0.0.1:0'\ndata_dir='{}'\nthreads=1\nadmin_token_env='GATEWAY_TEST_ADMIN'\n{extra}\n",dir.display())).unwrap();
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_gateway"))
            .args(["run", "--config", path.to_str().unwrap()])
            .env("GATEWAY_TEST_ADMIN", "fixture-admin-token")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .unwrap();
        let out = child.stdout.take().unwrap();
        let mut process = Self {
            child,
            base: String::new(),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                if line.contains("gateway ready") {
                    let _ = tx.send(line);
                }
            }
        });
        let line = rx
            .recv_timeout(std::time::Duration::from_secs(8))
            .expect("gateway readiness");
        let address = line
            .split("listen=")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap();
        process.base = format!("http://{address}");
        process
    }
    pub fn stop(&mut self) {
        unsafe { libc::kill(self.child.id() as i32, libc::SIGTERM) };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                return;
            }
            assert!(std::time::Instant::now() < deadline, "shutdown hung");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
