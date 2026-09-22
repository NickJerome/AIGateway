use gateway::{
    bytecode::compile, config::Config, engine::Invocation, limits::Limits, store::BytecodeStore,
    worker::Worker,
};
use std::time::{Duration, Instant};

fn config(extra: &str) -> String {
    format!("listen = \"127.0.0.1:0\"\ndata_dir = \"/tmp/test\"\nthreads = 1\n{extra}")
}
fn invoke(code: &gateway::bytecode::CompiledArtifact) -> String {
    Worker::new()
        .start_request()
        .unwrap()
        .invoke(
            code,
            Invocation {
                url: "/".into(),
                method: "GET".into(),
            },
        )
        .unwrap()
}
#[test]
fn unknown_config_field_is_rejected() {
    assert!(Config::from_toml(&config("max_in_fligth = 1\n")).is_err());
}
#[test]
fn zero_memory_limit_is_rejected() {
    assert!(Config::from_toml(&config("memory_limit_mb = 0\n")).is_err());
}
#[test]
fn zero_admission_limit_is_rejected() {
    assert!(Config::from_toml(&config("max_in_flight = 0\n")).is_err());
}
#[test]
fn overflowing_memory_limit_returns_error_not_panic() {
    let r = std::panic::catch_unwind(|| {
        Config::from_toml(&config("memory_limit_mb = 17592186044416\n"))
    });
    assert!(
        r.is_ok(),
        "overflow must be a configuration error, not a panic"
    );
    assert!(r.unwrap().is_err());
}
#[test]
fn warm_cache_respects_supplied_source_hash() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = BytecodeStore::new(dir.path().to_path_buf());
    store
        .install("p", "export default {fetch(){return 'old'}};")
        .unwrap();
    let code = store
        .get("p", "export default {fetch(){return 'new'}};")
        .unwrap();
    assert_eq!(invoke(&code), "new");
}
#[test]
fn metadata_must_bind_the_actual_bytecode() {
    let dir = tempfile::tempdir().unwrap();
    let source = "export default {fetch(){return 'expected'}};";
    BytecodeStore::new(dir.path().to_path_buf())
        .install("p", source)
        .unwrap();
    // A valid but unrelated local artifact simulates a mismatched two-file update.
    let other = compile("plugin.mjs", "export default {fetch(){return 'other'}};").unwrap();
    std::fs::write(dir.path().join("p/compiled/plugin.qbc"), other.code).unwrap();
    let mut store = BytecodeStore::new(dir.path().to_path_buf());
    let code = store.get("p", source).unwrap();
    assert_eq!(
        invoke(&code),
        "expected",
        "mismatch must trigger local recompilation"
    );
}
#[test]
fn plugin_name_cannot_escape_the_store_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("plugins");
    let mut store = BytecodeStore::new(root);
    let r = store.install("../outside", "export default {fetch(){return 'ok'}};");
    assert!(r.is_err(), "public install accepts an escaping name");
    assert!(!dir.path().join("outside").exists());
}
#[test]
fn unreadable_store_root_is_not_an_empty_store() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("plugins");
    std::fs::write(&root, b"not a directory").unwrap();
    assert!(BytecodeStore::new(root).check_root().is_err());
}
#[test]
fn infinite_microtask_chain_is_interrupted() {
    let code = compile(
        "plugin.mjs",
        "export default {async fetch(){while(true){await Promise.resolve();}}};",
    )
    .unwrap()
    .code;
    let worker = Worker::with_limits(Limits {
        slice_ms: 100,
        ..Limits::default()
    });
    let t = Instant::now();
    assert!(
        worker
            .start_request()
            .unwrap()
            .invoke(
                &code,
                Invocation {
                    url: "/".into(),
                    method: "GET".into()
                }
            )
            .is_err()
    );
    assert!(t.elapsed() < Duration::from_secs(2));
    assert_eq!(worker.in_flight(), 0);
}
