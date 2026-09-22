mod support;
use gateway::{
    http::Server,
    storage::Database,
    telemetry::{Telemetry, Trace},
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn stalled_upload(path: &str) {
    let dir = tempfile::tempdir().unwrap();
    support::install(
        dir.path(),
        "slow",
        "export default {fetch(){return new Response('ok')}}",
        "",
    )
    .await;
    let mut process = support::Process::start(
        dir.path(),
        "max_in_flight=1\n[http]\nidle_timeout_ms=100\ndrain_timeout_ms=200",
    );
    let mut socket = tokio::net::TcpStream::connect(process.base.trim_start_matches("http://"))
        .await
        .unwrap();
    socket.write_all(format!("POST {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer fixture-admin-token\r\nContent-Type: application/zip\r\nContent-Length: 100\r\n\r\nx").as_bytes()).await.unwrap();
    let mut response = vec![0; 4096];
    let n = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut response))
        .await
        .expect("stalled incoming upload must time out")
        .unwrap();
    assert!(String::from_utf8_lossy(&response[..n]).starts_with("HTTP/1.1 408"));
    let client = reqwest::Client::new();
    assert_eq!(
        client
            .get(format!("{}/slow/test", process.base))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .put(format!("{}/__gateway/plugins/slow/config", process.base))
            .bearer_auth("fixture-admin-token")
            .header("content-type", "application/json")
            .body("{}")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    process.stop();
}
#[tokio::test]
async fn stalled_plugin_upload_releases_admission() {
    stalled_upload("/slow/test").await;
}
#[tokio::test]
async fn stalled_admin_upload_releases_mutation_slot() {
    stalled_upload("/__gateway/plugins").await;
}

#[tokio::test]
async fn opening_database_does_not_interrupt_active_traces() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 1, 8).unwrap();
    let t = Telemetry::open(dir.path(), db, Default::default()).unwrap();
    let trace = Trace::begin(
        t.clone(),
        "test".into(),
        "GET".into(),
        "http://local/".into(),
        vec![],
    )
    .await
    .unwrap();
    t.flush().await.unwrap();
    let another = Database::open(dir.path(), 1, 8).unwrap();
    assert!(t.detail("test".into(), trace.id().into()).await.unwrap()["error"].is_null());
    drop(another);
    trace.finish(true, None).await.unwrap();
    t.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_fetch_records_upstream_attempt_without_ctx() {
    let upstream_dir = tempfile::tempdir().unwrap();
    support::install(
        upstream_dir.path(),
        "up",
        "export default {fetch(){return new Response('response fixture',{status:429})}}",
        "",
    )
    .await;
    let upstream = Server::start(support::config(upstream_dir.path()))
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    support::install(
        dir.path(),
        "proxy",
        &format!(
            "export default {{fetch(){{return fetch('http://{}/up')}}}}",
            upstream.address
        ),
        "",
    )
    .await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let r = reqwest::get(format!("http://{}/proxy", server.address))
        .await
        .unwrap();
    assert_eq!(r.status(), 429);
    assert_eq!(r.text().await.unwrap(), "response fixture");
    server.shutdown().await.unwrap();
    let upstream_address = upstream.address;
    upstream.shutdown().await.unwrap();
    let conn = rusqlite::Connection::open(dir.path().join("gateway.sqlite")).unwrap();
    let raw: String = conn
        .query_row("SELECT value FROM traces WHERE plugin='proxy'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let trace: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(trace["complete"], true);
    assert_eq!(trace["status"], 429);
    assert_eq!(trace["response_bytes"], 16);
    assert_eq!(trace["attempts"][0]["status"], 429);
    assert_eq!(
        trace["attempts"][0]["target"],
        format!("http://{}/up", upstream_address)
    );
}

#[tokio::test]
async fn plugin_exception_records_host_status_and_error() {
    let dir = tempfile::tempdir().unwrap();
    support::install(
        dir.path(),
        "broken",
        "export default {fetch(){throw new Error('fixture failure')}}",
        "",
    )
    .await;
    let server = Server::start(support::config(dir.path())).await.unwrap();
    let response = reqwest::get(format!("http://{}/broken", server.address))
        .await
        .unwrap();
    assert_eq!(response.status(), 500);
    response.bytes().await.unwrap();
    server.shutdown().await.unwrap();
    let conn = rusqlite::Connection::open(dir.path().join("gateway.sqlite")).unwrap();
    let raw: String = conn
        .query_row("SELECT value FROM traces", [], |r| r.get(0))
        .unwrap();
    let trace: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(trace["status"], 500);
    assert_eq!(trace["complete"], false);
    assert_eq!(trace["error"], "plugin execution failed");
}

#[tokio::test]
async fn trace_pagination_preserves_forward_api_and_scopes_backwards_pages() {
    use gateway::telemetry::ListCursor;
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 1, 8).unwrap();
    let t = Telemetry::open(dir.path(), db, Default::default()).unwrap();
    let mut ids = Vec::new();
    for plugin in ["a", "b", "a", "a"] {
        let trace = Trace::begin(
            t.clone(),
            plugin.into(),
            "GET".into(),
            "http://local/".into(),
            vec![],
        )
        .await
        .unwrap();
        ids.push(trace.id().to_owned());
        trace.finish(true, None).await.unwrap();
    }
    t.flush().await.unwrap();
    let first = t
        .list(
            Some("a".into()),
            gateway::telemetry::Kind::Traces,
            0,
            ListCursor::Before(i64::MAX),
            2,
        )
        .await
        .unwrap();
    assert_eq!(first.rows[0].request_id, ids[3]);
    assert_eq!(first.rows[1].request_id, ids[2]);
    assert!(first.has_more);
    let second = t
        .list(
            Some("a".into()),
            gateway::telemetry::Kind::Traces,
            0,
            ListCursor::Before(first.rows[1].id),
            2,
        )
        .await
        .unwrap();
    assert_eq!(second.rows[0].request_id, ids[0]);
    assert_eq!(second.rows.len(), 1);
    assert!(!second.has_more);
    let forward = t
        .list(
            Some("a".into()),
            gateway::telemetry::Kind::Traces,
            0,
            ListCursor::After(0),
            10,
        )
        .await
        .unwrap();
    assert_eq!(forward.rows[0].request_id, ids[0]);
    assert_eq!(forward.rows.len(), 3);
    assert!(
        t.list(
            Some("a".into()),
            gateway::telemetry::Kind::Traces,
            0,
            ListCursor::After(0),
            1001
        )
        .await
        .is_err()
    );
    t.shutdown().await.unwrap();
}
