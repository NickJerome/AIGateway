use gateway::bytecode::compile;
use gateway::engine::{Engine, Invocation};
use gateway::limits::Limits;
use gateway::worker::Worker;

fn call(engine: Engine, code: &gateway::bytecode::CompiledArtifact) -> anyhow::Result<String> {
    engine.invoke(
        code,
        Invocation {
            url: "/x".into(),
            method: "POST".into(),
        },
    )
}

#[test]
fn memory_bomb_is_rejected_at_allocation() {
    let code = compile("plugin.mjs", r#"export default { fetch() { const b = new ArrayBuffer(1e9); return `${b.byteLength}`; } };"#).unwrap().code;
    let worker = Worker::with_limits(Limits {
        memory_bytes: 8 * 1024 * 1024,
        ..Limits::default()
    });
    assert!(call(worker.start_request().unwrap(), &code).is_err());
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn memory_failure_preserves_a_peer_engine_and_next_request() {
    let bomb = compile(
        "plugin.mjs",
        r#"export default { fetch() { new ArrayBuffer(1e9); return "no"; } };"#,
    )
    .unwrap()
    .code;
    let ok = compile(
        "plugin.mjs",
        r#"export default { fetch() { return "still alive"; } };"#,
    )
    .unwrap()
    .code;
    let worker = Worker::with_limits(Limits {
        memory_bytes: 8 * 1024 * 1024,
        ..Limits::default()
    });
    let peer = worker.start_request().unwrap();
    assert!(call(worker.start_request().unwrap(), &bomb).is_err());
    assert_eq!(worker.in_flight(), 1);
    assert_eq!(call(peer, &ok).unwrap(), "still alive");
    assert_eq!(
        call(worker.start_request().unwrap(), &ok).unwrap(),
        "still alive"
    );
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn endless_loop_is_interrupted_within_the_slice() {
    let code = compile(
        "plugin.mjs",
        r#"export default { fetch() { while (true) {} } };"#,
    )
    .unwrap()
    .code;
    let worker = Worker::with_limits(Limits {
        slice_ms: 200,
        ..Limits::default()
    });
    let started = std::time::Instant::now();
    assert!(call(worker.start_request().unwrap(), &code).is_err());
    assert!(
        started.elapsed().as_millis() < 2_000,
        "{:?}",
        started.elapsed()
    );
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn each_request_gets_its_own_slice() {
    let code = compile("plugin.mjs", r#"export default { fetch() { const end = Date.now() + 120; while (Date.now() < end) {} return "slow but fine"; } };"#).unwrap().code;
    let worker = Worker::with_limits(Limits {
        slice_ms: 400,
        ..Limits::default()
    });
    for _ in 0..3 {
        assert_eq!(
            call(worker.start_request().unwrap(), &code).unwrap(),
            "slow but fine"
        );
    }
}

#[test]
fn waiting_before_invocation_does_not_consume_the_slice() {
    let code = compile("plugin.mjs", r#"export default {fetch(){return "ok"}};"#)
        .unwrap()
        .code;
    let worker = Worker::with_limits(Limits {
        slice_ms: 100,
        ..Limits::default()
    });
    let engine = worker.start_request().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert_eq!(call(engine, &code).unwrap(), "ok");
}

#[test]
fn invocation_error_returns_the_permit() {
    let code = compile(
        "plugin.mjs",
        r#"export default { fetch() { throw new Error("failed"); } };"#,
    )
    .unwrap()
    .code;
    let worker = Worker::with_limits(Limits {
        max_in_flight: 1,
        ..Limits::default()
    });
    for _ in 0..3 {
        assert!(call(worker.start_request().unwrap(), &code).is_err());
        assert_eq!(worker.in_flight(), 0);
    }
}
