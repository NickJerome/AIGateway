use gateway::{
    storage::Database,
    telemetry::{Config, Telemetry},
};
use serde_json::json;
#[tokio::test]
async fn count_time_flush_isolation_query_and_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 2, 16).unwrap();
    let t = Telemetry::open(dir.path(), db.clone(), Config::default()).unwrap();
    t.record("a", "request", json!({"account":"one","input":2}))
        .unwrap();
    assert_eq!(
        t.query("a".into(), 0, vec![]).await.unwrap()["rows"],
        json!([])
    );
    for _ in 1..64 {
        t.record("a", "request", json!({"account":"one","input":2}))
            .unwrap();
    }
    for _ in 0..100 {
        if !t.query("a".into(), 0, vec![]).await.unwrap()["rows"]
            .as_array()
            .unwrap()
            .is_empty()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let result = t
        .query("a".into(), 0, vec!["account".into()])
        .await
        .unwrap();
    assert_eq!(result["rows"][0]["count"], 64);
    assert_eq!(result["rows"][0]["sums"]["input"], 128.0);
    t.record("b", "request", json!({"account":"two","input":9}))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    assert_eq!(
        t.query("b".into(), 0, vec![]).await.unwrap()["rows"][0]["count"],
        1
    );
    t.record("a", "tail", json!({"input":1})).unwrap();
    t.shutdown().await.unwrap();
    let count = db
        .run(|c| Ok(c.query_row("SELECT count(*) FROM usage", [], |r| r.get::<_, i64>(0))?))
        .await
        .unwrap();
    assert_eq!(count, 66);
}
#[tokio::test]
async fn queue_limits_validation_and_write_failure_are_visible() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 1, 8).unwrap();
    let config = Config {
        queue_rows: 2,
        batch_rows: 64,
        flush_ms: 10000,
        ..Default::default()
    };
    let t = Telemetry::open(dir.path(), db.clone(), config).unwrap();
    assert!(t.record("a", "r", json!({"plugin_id":"b"})).is_err());
    assert!(t.record("a", "r", json!({"nested":{}})).is_err());
    t.record("a", "r", json!({"x":1})).unwrap();
    t.record("a", "r", json!({"x":2})).unwrap();
    assert!(t.record("a", "r", json!({"x":3})).is_err());
    db.run(|c|{c.execute_batch("CREATE TRIGGER fail_usage BEFORE INSERT ON usage BEGIN SELECT RAISE(ABORT,'injected usage failure'); END")?;Ok(())}).await.unwrap();
    assert!(t.flush().await.is_err());
    assert_eq!(
        t.query("a".into(), 0, vec![]).await.unwrap()["rows"],
        json!([])
    );
    db.run(|c| {
        c.execute_batch("DROP TRIGGER fail_usage")?;
        Ok(())
    })
    .await
    .unwrap();
    t.flush().await.unwrap();
    assert_eq!(
        t.query("a".into(), 0, vec![]).await.unwrap()["rows"][0]["count"],
        2
    );
    assert_eq!(t.counters()["failed_batches"], 1);
    t.shutdown().await.unwrap();
}

#[tokio::test]
async fn arbitrary_usage_names_do_not_overwrite_record_count() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 1, 8).unwrap();
    let t = Telemetry::open(dir.path(), db, Config::default()).unwrap();
    t.record("a", "r", json!({"count":7,"sums":"label","group":3}))
        .unwrap();
    t.flush().await.unwrap();
    let result = t.query("a".into(), 0, vec!["sums".into()]).await.unwrap();
    assert_eq!(
        result["rows"][0],
        json!({"group":{"sums":"label"},"count":1,"sums":{"count":7.0,"group":3.0}})
    );
    t.shutdown().await.unwrap();
}

#[tokio::test]
async fn raw_usage_preserves_reports_and_paginates_without_aggregation() {
    use gateway::telemetry::ListCursor;
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 1, 8).unwrap();
    let t = Telemetry::open(dir.path(), db, Config::default()).unwrap();
    t.record(
        "a",
        "first",
        json!({"model":"same", "input_tokens":3, "stream":false, "extra":null}),
    )
    .unwrap();
    t.record("b", "other", json!({"model":"same", "input_tokens":99}))
        .unwrap();
    t.record(
        "a",
        "second",
        json!({"model":"same", "input_tokens":0, "stream":true}),
    )
    .unwrap();
    t.flush().await.unwrap();
    let all = t
        .list(
            None,
            gateway::telemetry::Kind::Usage,
            0,
            ListCursor::Before(i64::MAX),
            2,
        )
        .await
        .unwrap();
    assert!(all.has_more);
    assert_eq!(all.rows[0].request_id, "second");
    assert_eq!(all.rows[0].plugin_id, "a");
    assert_eq!(all.rows[1].request_id, "other");
    assert_eq!(all.rows[1].plugin_id, "b");
    let older = t
        .list(
            None,
            gateway::telemetry::Kind::Usage,
            0,
            ListCursor::Before(all.rows[1].id),
            2,
        )
        .await
        .unwrap();
    assert!(!older.has_more);
    assert_eq!(older.rows.len(), 1);
    assert_eq!(older.rows[0].request_id, "first");
    assert_eq!(older.rows[0].plugin_id, "a");
    let first = t
        .list(
            Some("a".into()),
            gateway::telemetry::Kind::Usage,
            0,
            ListCursor::Before(i64::MAX),
            1,
        )
        .await
        .unwrap();
    assert!(first.has_more);
    assert_eq!(first.rows[0].request_id, "second");
    assert_eq!(
        first.rows[0].data,
        json!({"model":"same", "input_tokens":0, "stream":true})
    );
    let cursor = first.rows[0].id;
    let second = t
        .list(
            Some("a".into()),
            gateway::telemetry::Kind::Usage,
            0,
            ListCursor::Before(cursor),
            1,
        )
        .await
        .unwrap();
    assert!(!second.has_more);
    assert_eq!(second.rows[0].request_id, "first");
    assert_eq!(
        second.rows[0].data,
        json!({"model":"same", "input_tokens":3, "stream":false, "extra":null})
    );
    assert!(
        t.list(
            Some("a".into()),
            gateway::telemetry::Kind::Usage,
            i64::MAX,
            ListCursor::After(0),
            10
        )
        .await
        .unwrap()
        .rows
        .is_empty()
    );
    assert_eq!(
        t.query("a".into(), 0, vec![]).await.unwrap()["rows"][0]["count"],
        2
    );
    t.shutdown().await.unwrap();
}
