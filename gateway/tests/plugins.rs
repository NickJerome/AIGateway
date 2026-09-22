mod support;
use gateway::{
    plugins::{Manifest, Matched, Overrides, PluginManager, PluginRecord, route_matches},
    storage::Database,
};
#[tokio::test]
async fn install_config_disable_uninstall_and_reinstall_preserve_data() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 1, 8).unwrap();
    let mut manager =
        PluginManager::open(dir.path().join("plugins"), db.clone(), Default::default())
            .await
            .unwrap();
    let code = "export default {fetch(){return new Response('ok')}}";
    let extra = "[[kv_namespaces]]\nbinding='CONFIG'\nid='stable'";
    manager
        .install(support::package("hello", code, extra))
        .await
        .unwrap();
    let routes = manager.routes();
    assert!(matches!(routes.lookup("/hello/a"), Matched::Ready(_)));
    assert!(matches!(routes.lookup("/hello2"), Matched::Missing));
    let kv = db.namespace("stable").await.unwrap();
    kv.put("x".into(), b"keep".to_vec(), None).await.unwrap();
    let mut overrides = Overrides::default();
    overrides
        .vars
        .insert("SECRET".into(), "hidden-value".into());
    manager.configure("hello", overrides).await.unwrap();
    assert!(
        !manager
            .detail("hello")
            .unwrap()
            .to_string()
            .contains("hidden-value")
    );
    manager.enable("hello", false).await.unwrap();
    assert!(matches!(routes.lookup("/hello"), Matched::Unavailable));
    manager.uninstall("hello").await.unwrap();
    assert!(matches!(routes.lookup("/hello"), Matched::Missing));
    assert_eq!(kv.get("x".into()).await.unwrap(), Some(b"keep".to_vec()));
    manager
        .install(support::package("hello", code, extra))
        .await
        .unwrap();
    match routes.lookup("/hello") {
        Matched::Ready(p) => assert_eq!(p.vars["SECRET"], "hidden-value"),
        _ => panic!("not enabled"),
    }
}
#[tokio::test]
async fn invalid_archive_does_not_stop_old_plugin_failed_compile_does() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 1, 8).unwrap();
    let mut manager = PluginManager::open(dir.path().join("plugins"), db, Default::default())
        .await
        .unwrap();
    manager
        .install(support::package(
            "p",
            "export default {fetch(){return new Response('ok')}}",
            "",
        ))
        .await
        .unwrap();
    assert!(
        manager
            .install(support::zip_files(&[("../escape", b"bad")]))
            .await
            .is_err()
    );
    assert!(matches!(manager.routes().lookup("/p"), Matched::Ready(_)));
    assert!(!dir.path().join("escape").exists());
    assert!(
        manager
            .install(support::package("p", "export default {", ""))
            .await
            .is_err()
    );
    assert!(matches!(
        manager.routes().lookup("/p"),
        Matched::Unavailable
    ));
    manager
        .install(support::package(
            "p",
            "export default {fetch(){return new Response('fixed')}}",
            "",
        ))
        .await
        .unwrap();
    assert!(matches!(manager.routes().lookup("/p"), Matched::Ready(_)));
}
#[test]
fn manifest_and_route_contract() {
    assert!(route_matches("/a/*", "/a"));
    assert!(route_matches("/a/*", "/a/b?q=x"));
    assert!(!route_matches("/a/*", "/ab"));
    assert!(!route_matches("/a", "/a/b"));
    for extra in [
        "compatibility_flags=['nodejs_compat']",
        "compatibility_date='2026-02-30'",
        "routes=['/__gateway/*']",
        "main='../x.js'",
    ] {
        let base = if extra.starts_with("main=") {
            "name='p'\n"
        } else {
            "name='p'\nmain='x.js'\n"
        };
        assert!(
            Manifest::parse(&format!("{base}{extra}")).is_err(),
            "{extra}"
        );
    }
}
#[tokio::test]
async fn unbundled_static_import_is_rejected_at_install() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 1, 8).unwrap();
    let mut manager = PluginManager::open(dir.path().join("plugins"), db, Default::default())
        .await
        .unwrap();
    assert!(
        manager
            .install(support::package(
                "p",
                "import x from 'node:fs'; export default {fetch(){return new Response(x)}}",
                ""
            ))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn restart_marks_interrupted_install_failed_and_missing_source_visible() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 1, 8).unwrap();
    let root = dir.path().join("plugins");
    let mut manager = PluginManager::open(root.clone(), db.clone(), Default::default())
        .await
        .unwrap();
    manager
        .install(support::package(
            "p",
            "export default {fetch(){return new Response('ok')}}",
            "",
        ))
        .await
        .unwrap();
    let mut record = PluginRecord::list(&db).await.unwrap().remove(0);
    record.state = "installing".into();
    record.save(&db).await.unwrap();
    drop(manager);
    drop(db);
    let db = Database::open(dir.path(), 1, 8).unwrap();
    let mut manager = PluginManager::open(root.clone(), db.clone(), Default::default())
        .await
        .unwrap();
    assert_eq!(manager.detail("p").unwrap()["state"], "failed");
    manager.enable("p", true).await.unwrap();
    drop(manager);
    std::fs::remove_file(root.join("p/dist/worker.mjs")).unwrap();
    let manager = PluginManager::open(root, db, Default::default())
        .await
        .unwrap();
    assert_eq!(manager.detail("p").unwrap()["state"], "failed");
    assert!(
        manager.detail("p").unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("read plugin main")
    );
}
#[tokio::test]
async fn zip_reserved_paths_links_conflicts_and_actual_expansion_limits() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 1, 8).unwrap();
    let mut manager = PluginManager::open(
        dir.path().join("plugins"),
        db,
        gateway::plugins::ZipLimits {
            expanded: 1024,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    for name in [
        "/absolute",
        "a/../escape",
        "a\\escape",
        "compiled/plugin.qbc",
        "C:/escape",
    ] {
        assert!(
            manager
                .install(support::zip_files(&[(name, b"bad")]))
                .await
                .is_err()
        );
    }
    assert!(
        manager
            .install(support::zip_files(&[("a", b"x"), ("a/b", b"x")]))
            .await
            .is_err()
    );
    assert!(
        manager
            .install(support::zip_files(&[("big", &vec![0; 1025])]))
            .await
            .is_err()
    );
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .add_symlink(
            "link",
            "../../outside",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    assert!(
        manager
            .install(writer.finish().unwrap().into_inner())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn configuration_read_requires_administrator_and_is_not_cached() {
    let dir = tempfile::tempdir().unwrap();
    let mut process = support::Process::start(dir.path(), "");
    let client = reqwest::Client::new();
    let admin = "fixture-admin-token";
    let base = format!("{}/__gateway/plugins", process.base);
    let package = support::package(
        "settings",
        "export default {fetch(){return new Response('ok')}}",
        "[vars]\nMODEL='default-model'\nAPI_KEY='fixture-secret'",
    );
    assert_eq!(
        client
            .post(&base)
            .bearer_auth(admin)
            .header("content-type", "application/zip")
            .body(package)
            .send()
            .await
            .unwrap()
            .status(),
        201
    );
    let url = format!("{base}/settings/config");
    assert_eq!(client.get(&url).send().await.unwrap().status(), 401);
    let response = client.get(&url).bearer_auth(admin).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let config: serde_json::Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
    assert_eq!(config["defaults"]["MODEL"], "default-model");
    assert_eq!(config["defaults"]["API_KEY"], "fixture-secret");
    let listed = client
        .get(&base)
        .bearer_auth(admin)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!listed.contains("fixture-secret"));
    let response = client
        .post(format!("{}/__gateway/keys", process.base))
        .bearer_auth(admin)
        .header("content-type", "application/json")
        .body(r#"{"name":"settings-test"}"#)
        .send()
        .await
        .unwrap();
    let key: serde_json::Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
    assert_eq!(
        client
            .get(&url)
            .bearer_auth(key["key"].as_str().unwrap())
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        client
            .get(format!("{base}/absent/config"))
            .bearer_auth(admin)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    process.stop();
}

#[tokio::test]
async fn invalid_config_keeps_running_plugin_and_saved_overrides() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 1, 8).unwrap();
    let mut manager =
        PluginManager::open(dir.path().join("plugins"), db.clone(), Default::default())
            .await
            .unwrap();
    manager.install(support::package("stable", "export default {fetch(){return new Response('ok')}}", "[[kv_namespaces]]\nbinding='DATA'\nid='stable-data'\n[[unsafe.bindings]]\nname='USAGE'\ntype='usage'")).await.unwrap();
    let routes = manager.routes();
    let Matched::Ready(original) = routes.lookup("/stable") else {
        panic!("plugin not running")
    };
    for (key, value, binding) in [
        ("IDENTITY", "fake", false),
        ("DATA", "conflict", false),
        ("USAGE", "conflict", false),
        ("missing", "data", true),
        ("DATA", "", true),
    ] {
        let mut config = Overrides::default();
        if binding {
            config.bindings.insert(key.into(), value.into());
        } else {
            config.vars.insert(key.into(), value.into());
        }
        assert!(manager.configure("stable", config).await.is_err());
        let Matched::Ready(current) = routes.lookup("/stable") else {
            panic!("invalid config stopped healthy plugin")
        };
        assert!(std::sync::Arc::ptr_eq(&original, &current));
        let record = PluginRecord::list(&db).await.unwrap().remove(0);
        assert!(record.overrides.vars.is_empty());
        assert!(record.overrides.bindings.is_empty());
        assert!(record.enabled);
    }
}
