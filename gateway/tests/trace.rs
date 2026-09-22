mod support;
use gateway::{
    http::Server,
    storage::Database,
    telemetry::{Config, Telemetry, Trace},
};
use sha2::{Digest, Sha256};
#[tokio::test]
async fn chunks_hash_retention_and_plugin_isolation() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 2, 16).unwrap();
    let t = Telemetry::open(
        dir.path(),
        db.clone(),
        Config {
            retain_rows: 1,
            ..Default::default()
        },
    )
    .unwrap();
    let trace = Trace::begin(
        t.clone(),
        "a".into(),
        "POST".into(),
        "http://local/".into(),
        vec![],
    )
    .await
    .unwrap();
    let data = vec![b'x'; 478000];
    trace
        .chunk(0, bytes::Bytes::from(data.clone()))
        .await
        .unwrap();
    trace
        .chunk(1, bytes::Bytes::from_static(b"response"))
        .await
        .unwrap();
    trace.finish(true, None).await.unwrap();
    t.flush().await.unwrap();
    let detail = t.detail("a".into(), trace.id().into()).await.unwrap();
    assert_eq!(detail["request_hash"], hex::encode(Sha256::digest(&data)));
    assert!(t.detail("b".into(), trace.id().into()).await.is_err());
    let mut saved = vec![];
    for seq in 0.. {
        match t
            .chunk("a".into(), trace.id().into(), "request".into(), seq)
            .await
            .unwrap()
        {
            Some(chunk) => saved.extend(chunk),
            None => break,
        }
    }
    assert_eq!(saved, data);
    t.record("a", "usage", serde_json::json!({"input":9}))
        .unwrap();
    let newer = Trace::begin(
        t.clone(),
        "a".into(),
        "GET".into(),
        "http://local/".into(),
        vec![],
    )
    .await
    .unwrap();
    newer.finish(false, Some("cancelled".into())).await.unwrap();
    t.flush().await.unwrap();
    assert!(t.detail("a".into(), trace.id().into()).await.is_err());
    assert_eq!(
        t.query("a".into(), 0, vec![]).await.unwrap()["rows"][0]["sums"]["input"],
        9.0
    );
    t.shutdown().await.unwrap();
}
#[tokio::test]
async fn real_http_usage_and_complete_trace() {
    let dir = tempfile::tempdir().unwrap();
    support::install(dir.path(),"observed",r#"export default {async fetch(r,env){env.USAGE.record({model:'m',input:3});console.info('request recorded');return new Response(r.body);}}"#,"[[unsafe.bindings]]\nname='USAGE'\ntype='usage'").await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let body = vec![b'z'; 478000];
    let response = reqwest::Client::new()
        .post(format!("http://{}/observed/", server.address))
        .body(body.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.bytes().await.unwrap().as_ref(), body);
    server.shutdown().await.unwrap();
    let c = rusqlite::Connection::open(dir.path().join("gateway.sqlite")).unwrap();
    let trace: String = c
        .query_row("SELECT value FROM traces", [], |r| r.get(0))
        .unwrap();
    let trace: serde_json::Value = serde_json::from_str(&trace).unwrap();
    assert_eq!(trace["complete"], true);
    assert_eq!(trace["response_hash"], hex::encode(Sha256::digest(&body)));
    let usage: i64 = c
        .query_row("SELECT count(*) FROM usage", [], |r| r.get(0))
        .unwrap();
    assert_eq!(usage, 1);
    let logs: i64 = c
        .query_row("SELECT count(*) FROM logs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(logs, 1);
}

#[tokio::test]
async fn per_plugin_body_switch_and_log_byte_retention() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 1, 8).unwrap();
    let t = Telemetry::open(
        dir.path(),
        db.clone(),
        Config {
            retain_bytes: 2000,
            ..Default::default()
        },
    )
    .unwrap();
    let trace = Trace::begin_configured(
        t.clone(),
        "a".into(),
        "POST".into(),
        "http://local/".into(),
        vec![],
        Some(false),
    )
    .await
    .unwrap();
    trace
        .chunk(0, bytes::Bytes::from_static(b"private fixture"))
        .await
        .unwrap();
    trace.finish(true, None).await.unwrap();
    t.flush().await.unwrap();
    assert_eq!(
        t.detail("a".into(), trace.id().into()).await.unwrap()["body_recorded"],
        false
    );
    assert!(
        t.chunk("a".into(), trace.id().into(), "request".into(), 0)
            .await
            .unwrap()
            .is_none()
    );
    t.record("a", "r", serde_json::json!({"value":1})).unwrap();
    for _ in 0..10 {
        t.log("a", "r", "info", "x".repeat(900));
    }
    t.flush().await.unwrap();
    let bytes = db
        .run(|c| {
            Ok(c.query_row(
                "SELECT COALESCE(sum(length(CAST(value AS BLOB))),0) FROM logs",
                [],
                |r| r.get::<_, i64>(0),
            )?)
        })
        .await
        .unwrap();
    assert!(bytes <= 2000);
    assert_eq!(
        t.query("a".into(), 0, vec![]).await.unwrap()["rows"][0]["count"],
        1
    );
    t.shutdown().await.unwrap();
}
