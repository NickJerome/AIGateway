mod support;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const READY_TIMEOUT: Duration = Duration::from_secs(10);

/// Reaps the child on every exit path: a failed assertion or a panic drops
/// the guard, which kills (if still running) and waits, so the test suite
/// never leaves a gateway process behind.
struct ChildGuard(Child);

impl ChildGuard {
    fn terminate(&mut self) -> i32 {
        unsafe { libc::kill(self.0.id() as i32, libc::SIGTERM) };
        self.0.wait().unwrap().code().unwrap_or(-1)
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            unsafe { libc::kill(self.0.id() as i32, libc::SIGKILL) };
        }
        let _ = self.0.wait();
    }
}

/// Reads stdout on a dedicated thread and waits on a channel with a timeout,
/// so readiness is truly bounded: a blocking read_line could otherwise hang
/// past the deadline when the child stops producing output.
fn start(config: &str, dir: &std::path::Path) -> (ChildGuard, String) {
    let config_path = dir.join("gateway.toml");
    std::fs::write(&config_path, config).unwrap();
    let mut child = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_gateway"))
            .args(["run", "--config", config_path.to_str().unwrap()])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut output = String::new();
    while let Ok(line) = rx.recv_timeout(READY_TIMEOUT) {
        let line = line.unwrap();
        output.push_str(&line);
        output.push('\n');
        if line.contains("gateway ready") {
            return (child, output);
        }
    }
    panic!("binary did not report readiness within {READY_TIMEOUT:?}; output so far:\n{output}");
}

#[test]
fn starts_with_the_configured_thread_count() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = format!(
        "listen = \"127.0.0.1:0\"\ndata_dir = \"{}\"\nthreads = 3\n",
        dir.path().display()
    );
    let (mut child, ready) = start(&cfg, dir.path());
    assert!(ready.contains("threads=3"), "readiness line was: {ready}");
    assert_eq!(child.terminate(), 0, "SIGTERM must exit cleanly");
}

#[test]
fn derives_thread_count_when_omitted() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = format!(
        "listen = \"127.0.0.1:0\"\ndata_dir = \"{}\"\n",
        dir.path().display()
    );
    let (mut child, ready) = start(&cfg, dir.path());
    let n: usize = ready
        .split("threads=")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(n >= 1);
    assert_eq!(child.terminate(), 0);
}

#[test]
fn missing_config_fails_loudly() {
    let out = Command::new(env!("CARGO_BIN_EXE_gateway"))
        .args(["run", "--config", "/nonexistent/gateway.toml"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("gateway.toml"));
}

#[test]
fn unknown_subcommand_is_rejected() {
    let out = Command::new(env!("CARGO_BIN_EXE_gateway"))
        .arg("eval")
        .output()
        .unwrap();
    assert!(!out.status.success(), "只有 run 一个子命令");
}

#[test]
fn warms_installed_plugins_at_startup() {
    let dir = tempfile::tempdir().unwrap();
    futures::executor::block_on(support::install(
        dir.path(),
        "codex-pool",
        "export default {fetch(){return new Response('ok')}}",
        "",
    ));
    std::fs::remove_dir_all(dir.path().join("plugins/codex-pool/compiled")).unwrap();

    let cfg = format!(
        "listen = \"127.0.0.1:0\"\ndata_dir = \"{}\"\nthreads = 1\n",
        dir.path().display()
    );
    let (mut child, ready) = start(&cfg, dir.path());
    assert!(ready.contains("plugins=1"), "启动要预热已安装插件：{ready}");
    assert!(
        ready.contains("recompiled=1"),
        "首次启动没有产物，应当编译一次：{ready}"
    );
    assert_eq!(child.terminate(), 0);

    // 第二次启动：产物还在且匹配，必须读盘而不是重编译
    let (mut child, ready) = start(&cfg, dir.path());
    assert!(
        ready.contains("loaded=1") && ready.contains("recompiled=0"),
        "重启应命中磁盘产物：{ready}"
    );
    assert_eq!(child.terminate(), 0);
}

#[test]
fn limit_overrides_are_read_from_config() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = format!(
        "listen = \"127.0.0.1:0\"\ndata_dir = \"{}\"\nthreads = 1\nmemory_limit_mb = 16\nmax_in_flight = 4\n",
        dir.path().display()
    );
    let (mut child, ready) = start(&cfg, dir.path());
    assert!(ready.contains("threads=1"), "{ready}");
    assert_eq!(child.terminate(), 0, "带可选覆盖的配置必须能正常启动");
}

/// Runs the binary to completion with the given arguments.
fn run_with_args(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_gateway"))
        .args(args)
        .output()
        .unwrap()
}

fn assert_rejected_without_ready(out: &std::process::Output, arg: &str) {
    assert_eq!(out.status.code(), Some(1), "rejected arguments must exit 1");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("gateway ready"),
        "parse failure must not print ready: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&format!("unexpected argument \"{arg}\"")),
        "stderr must name the offending argument: {stderr}"
    );
    assert!(
        stderr.contains("usage: gateway run [--config <path>]"),
        "{stderr}"
    );
}

#[test]
fn unknown_flag_is_rejected() {
    assert_rejected_without_ready(&run_with_args(&["run", "--bogus"]), "--bogus");
}

#[test]
fn duplicate_config_flag_is_rejected() {
    assert_rejected_without_ready(
        &run_with_args(&["run", "--config", "a.toml", "--config", "b.toml"]),
        "--config",
    );
}

#[test]
fn extra_positional_argument_is_rejected() {
    assert_rejected_without_ready(&run_with_args(&["run", "extra"]), "extra");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn confirmed_kv_and_durable_writes_survive_sigkill() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"crash",r#"import {DurableObject} from 'cloudflare:workers';export class State extends DurableObject {async read(){return await this.ctx.storage.get('v');}async write(){await this.ctx.storage.put('v',{ok:42});}}export default {async fetch(r,env){const s=env.STATE.get(env.STATE.idFromName('one'));if(r.method==='POST'){await env.KV.put('v','confirmed');await s.write();return new Response('confirmed');}return Response.json({kv:await env.KV.get('v'),do:await s.read()});}}"#,"[[kv_namespaces]]\nbinding='KV'\nid='crash-kv'\n[[durable_objects.bindings]]\nname='STATE'\nclass_name='State'\n[[migrations]]\ntag='v1'\nnew_sqlite_classes=['State']").await;
    let mut process = support::Process::start(dir.path(), "");
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/crash/", process.base))
        .send()
        .await
        .unwrap();
    assert_eq!(response.text().await.unwrap(), "confirmed");
    process.child.kill().unwrap();
    process.child.wait().unwrap();
    let mut process = support::Process::start(dir.path(), "");
    let result = client
        .get(format!("{}/crash/", process.base))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(result, r#"{"kv":"confirmed","do":{"ok":42}}"#);
    process.stop();
}
