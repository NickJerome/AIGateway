use gateway::bytecode::compile;
use gateway::engine::Invocation;
use gateway::limits::Limits;
use gateway::worker::Worker;
use std::time::{Duration, Instant};

fn call(worker: &Worker, code: &gateway::bytecode::CompiledArtifact) -> anyhow::Result<String> {
    worker.start_request()?.invoke(
        code,
        Invocation {
            url: "/x".into(),
            method: "POST".into(),
        },
    )
}

#[test]
fn pending_microtasks_do_not_leak_into_the_next_request() {
    let code = compile(
        "plugin.mjs",
        r#"
export default {
  fetch() {
    Promise.resolve().then(() => { globalThis.__late = (globalThis.__late ?? 0) + 1; });
    return `${globalThis.__late ?? 0}`;
  }
};
"#,
    )
    .unwrap()
    .code;
    let worker = Worker::new();
    assert_eq!(call(&worker, &code).unwrap(), "0");
    assert_eq!(call(&worker, &code).unwrap(), "0");
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn job_after_response_must_also_be_interrupted() {
    let code = compile(
        "plugin.mjs",
        r#"
export default {
  fetch() {
    Promise.resolve().then(() => { while (true) {} });
    return "returned early";
  }
};
"#,
    )
    .unwrap()
    .code;
    let worker = Worker::with_limits(Limits {
        slice_ms: 200,
        ..Limits::default()
    });
    let started = Instant::now();
    assert!(call(&worker, &code).is_err());
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(worker.in_flight(), 0);
    let ok = compile("plugin.mjs", r#"export default {fetch(){return "ok"}};"#)
        .unwrap()
        .code;
    assert_eq!(call(&worker, &ok).unwrap(), "ok");
}

#[test]
fn failure_discards_remaining_jobs_without_running_them() {
    let code = compile(
        "plugin.mjs",
        r#"
export default {
  fetch() {
    Promise.resolve().then(() => { while (true) {} });
    throw new Error("request failed");
  }
};
"#,
    )
    .unwrap()
    .code;
    let worker = Worker::with_limits(Limits {
        slice_ms: 200,
        ..Limits::default()
    });
    let err = call(&worker, &code).unwrap_err().to_string();
    assert!(err.contains("request failed"), "{err}");
    assert!(
        !err.contains("slice exceeded"),
        "failed requests must not drain jobs"
    );
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn cyclic_request_heaps_are_reclaimed() {
    // An isolated process prevents unrelated parallel tests from skewing RSS.
    if std::env::var_os("GATEWAY_HEAP_CHILD").is_none() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "cyclic_request_heaps_are_reclaimed",
                "--nocapture",
            ])
            .env("GATEWAY_HEAP_CHILD", "1")
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        // Every failure path kills and waits, so the child is always reaped.
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(err) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("try_wait failed: {err}");
                }
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("cyclic heap regression exceeded 10 seconds");
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(status.success());
        return;
    }
    fn rss() -> usize {
        let text = std::fs::read_to_string("/proc/self/statm").unwrap();
        let pages: usize = text.split_whitespace().nth(1).unwrap().parse().unwrap();
        pages * unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize }
    }
    let code = compile(
        "plugin.mjs",
        r#"
export default {fetch(){
  const object = {buffer: new ArrayBuffer(1024 * 1024)};
  new Uint8Array(object.buffer).fill(7);
  object.self = object;
  globalThis.held = object;
  return "ok";
}};
"#,
    )
    .unwrap()
    .code;
    let worker = Worker::new();
    for _ in 0..30 {
        assert_eq!(call(&worker, &code).unwrap(), "ok");
    }
    let before = rss();
    for _ in 0..300 {
        assert_eq!(call(&worker, &code).unwrap(), "ok");
    }
    let after = rss();
    assert_eq!(worker.in_flight(), 0);
    assert!(
        after <= before + 16 * 1024 * 1024,
        "RSS grew from {before} to {after}"
    );
}
