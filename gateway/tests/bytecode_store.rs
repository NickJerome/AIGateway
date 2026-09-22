use gateway::bytecode::{BUILD_ID, compile, source_sha256};

const HELLO: &str = r#"export default { fetch() { return "ok"; } };"#;

#[test]
fn compiles_to_bytecode_with_metadata() {
    let art = compile("plugin.mjs", HELLO).unwrap();
    assert!(!art.code.is_empty());
    assert_eq!(art.source_sha256, source_sha256(HELLO));
    assert_eq!(art.build_id, BUILD_ID);
}

#[test]
fn compile_is_deterministic() {
    assert_eq!(
        compile("plugin.mjs", HELLO).unwrap().code,
        compile("plugin.mjs", HELLO).unwrap().code
    );
}

#[test]
fn syntax_error_is_reported_not_panicked() {
    let err = compile("plugin.mjs", "export default {").unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("plugin.mjs"),
        "error must name the module: {msg}"
    );
}
use gateway::store::BytecodeStore;
use std::fs;

#[test]
fn install_writes_artifact_and_meta() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = BytecodeStore::new(dir.path().to_path_buf());
    let code = store.install("codex-pool", HELLO).unwrap();
    assert!(!code.is_empty());
    assert!(dir.path().join("codex-pool/compiled/plugin.qbc").exists());
    assert!(dir.path().join("codex-pool/compiled/meta.toml").exists());
}

#[test]
fn memory_hit_avoids_disk_and_recompile() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = BytecodeStore::new(dir.path().to_path_buf());
    store.install("p", HELLO).unwrap();
    store.get("p", HELLO).unwrap();
    store.get("p", HELLO).unwrap();
    assert_eq!(store.stats(), (2, 0, 1), "install 编一次，随后两次内存命中");
}

#[test]
fn cold_start_reads_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = BytecodeStore::new(dir.path().to_path_buf());
    store.install("p", HELLO).unwrap();
    let mut fresh = BytecodeStore::new(dir.path().to_path_buf()); // 模拟重启
    fresh.get("p", HELLO).unwrap();
    assert_eq!(fresh.stats(), (0, 1, 0));
}

#[test]
fn stale_build_id_triggers_recompile() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = BytecodeStore::new(dir.path().to_path_buf());
    store.install("p", HELLO).unwrap();
    let meta_path = dir.path().join("p/compiled/meta.toml");
    let meta = fs::read_to_string(&meta_path)
        .unwrap()
        .replace(BUILD_ID, "qjs-ng/ancient");
    fs::write(&meta_path, meta).unwrap();
    let mut fresh = BytecodeStore::new(dir.path().to_path_buf());
    fresh.get("p", HELLO).unwrap();
    assert_eq!(fresh.stats().2, 1, "构建标识不兼容必须重编译");
}

#[test]
fn changed_source_triggers_recompile() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = BytecodeStore::new(dir.path().to_path_buf());
    store.install("p", HELLO).unwrap();
    let mut fresh = BytecodeStore::new(dir.path().to_path_buf());
    fresh
        .get("p", r#"export default { fetch() { return "changed"; } };"#)
        .unwrap();
    assert_eq!(fresh.stats().2, 1, "源码哈希不匹配必须重编译");
}

#[test]
fn truncated_hash_mismatch_and_half_update_are_cache_misses() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = BytecodeStore::new(dir.path().to_path_buf());
    store.install("p", HELLO).unwrap();
    std::fs::write(dir.path().join("p/compiled/plugin.qbc"), b"bad").unwrap();
    let mut fresh = BytecodeStore::new(dir.path().to_path_buf());
    fresh.get("p", HELLO).unwrap();
    assert_eq!(fresh.stats().2, 1);
    std::fs::write(dir.path().join("p/compiled/meta.toml"), b"format_version=1").unwrap();
    let mut fresh = BytecodeStore::new(dir.path().to_path_buf());
    fresh.get("p", HELLO).unwrap();
    assert_eq!(fresh.stats().2, 1);
}
