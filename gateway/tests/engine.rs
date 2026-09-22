use gateway::bytecode::compile;
use gateway::engine::Invocation;
use gateway::worker::Worker;

fn call(
    worker: &Worker,
    code: &gateway::bytecode::CompiledArtifact,
    url: &str,
) -> anyhow::Result<String> {
    worker.start_request()?.invoke(
        code,
        Invocation {
            url: url.to_string(),
            method: "POST".to_string(),
        },
    )
}

#[test]
fn calls_default_export_fetch() {
    let code = compile(
        "plugin.mjs",
        r#"
export default {
  fetch(request, env, ctx) { return `${request.method} ${request.url}`; }
};
"#,
    )
    .unwrap()
    .code;
    let worker = Worker::new();
    assert_eq!(
        call(&worker, &code, "/codex/responses").unwrap(),
        "POST /codex/responses"
    );
}

#[test]
fn module_state_does_not_leak_between_requests() {
    let code = compile(
        "plugin.mjs",
        r#"
let calls = 0;
globalThis.leaked = (globalThis.leaked ?? 0) + 1;
export default { fetch() { calls++; return `${calls}/${globalThis.leaked}`; } };
"#,
    )
    .unwrap()
    .code;
    let worker = Worker::new();
    assert_eq!(call(&worker, &code, "/a").unwrap(), "1/1");
    assert_eq!(call(&worker, &code, "/b").unwrap(), "1/1");
}

#[test]
fn awaited_async_fetch_resolves() {
    let code = compile(
        "plugin.mjs",
        r#"
export default { async fetch() { await null; return "async ok"; } };
"#,
    )
    .unwrap()
    .code;
    let worker = Worker::new();
    assert_eq!(call(&worker, &code, "/a").unwrap(), "async ok");
}

#[test]
fn thrown_error_becomes_rust_error_with_message() {
    let code = compile(
        "plugin.mjs",
        r#"
export default { fetch() { throw new Error("boom in plugin"); } };
"#,
    )
    .unwrap()
    .code;
    let worker = Worker::new();
    let err = format!("{:#}", call(&worker, &code, "/a").unwrap_err());
    assert!(err.contains("boom in plugin"), "{err}");
}

#[test]
fn missing_default_export_is_an_error() {
    let code = compile("plugin.mjs", "export const notDefault = 1;")
        .unwrap()
        .code;
    let worker = Worker::new();
    let err = format!("{:#}", call(&worker, &code, "/a").unwrap_err());
    assert!(err.contains("default"), "{err}");
}

#[test]
fn module_initialization_rejection_is_reported() {
    let code = compile(
        "plugin.mjs",
        r#"throw new Error("init failed"); export default {fetch(){return "wrong"}};"#,
    )
    .unwrap()
    .code;
    let worker = Worker::new();
    let err = call(&worker, &code, "/x").unwrap_err().to_string();
    assert!(err.contains("init failed"), "{err}");
    assert_eq!(worker.in_flight(), 0);
}
#[test]
fn top_level_await_finishes_before_fetch() {
    let code = compile(
        "plugin.mjs",
        r#"const value = await Promise.resolve("ready"); export default {fetch(){return value}};"#,
    )
    .unwrap()
    .code;
    assert_eq!(call(&Worker::new(), &code, "/x").unwrap(), "ready");
}
