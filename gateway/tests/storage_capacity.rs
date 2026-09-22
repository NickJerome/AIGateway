use gateway::{
    storage::Database,
    telemetry::{Config, Telemetry},
};
use serde_json::json;
#[tokio::test]
#[ignore = "manual release SQLite FULL/NORMAL and checkpoint measurement"]
async fn full_normal_and_checkpoint_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 1, 128).unwrap();
    let kv = db.namespace("capacity").await.unwrap();
    for kind in ["kv", "do"] {
        let mut times = vec![];
        for i in 0..200 {
            let started = std::time::Instant::now();
            if kind == "kv" {
                kv.put(i.to_string(), vec![b'x'; 512], None).await.unwrap();
            } else {
                db.do_storage(
                    "capacity".into(),
                    "object".into(),
                    "put".into(),
                    i.to_string(),
                    Some("x".repeat(512)),
                )
                .await
                .unwrap();
            }
            times.push(started.elapsed().as_micros());
        }
        times.sort();
        println!(
            "STORAGE_CAPACITY {}",
            json!({"kind":kind,"synchronous":"FULL","bytes":512,"writes":200,"commit_us":{"p50":times[99],"p95":times[189],"p99":times[197]},"mean_us":times.iter().sum::<u128>()/200})
        );
    }
    let telemetry = Telemetry::open(dir.path(), db.clone(), Config::default()).unwrap();
    let started = std::time::Instant::now();
    for i in 0..4096 {
        telemetry
            .record("capacity", &i.to_string(), json!({"input":512}))
            .unwrap();
    }
    telemetry.flush().await.unwrap();
    let ms = started.elapsed().as_secs_f64() * 1000.;
    let before = std::fs::metadata(dir.path().join("gateway.sqlite-wal"))
        .unwrap()
        .len();
    let started = std::time::Instant::now();
    let checkpoint = db
        .run(|c| {
            Ok(c.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?)
        })
        .await
        .unwrap();
    let payload = db
        .run(|c| {
            Ok(c.query_row(
                "SELECT sum(length(CAST(value AS BLOB))) FROM usage",
                [],
                |r| r.get::<_, i64>(0),
            )?)
        })
        .await
        .unwrap();
    println!(
        "STORAGE_CAPACITY {}",
        json!({"kind":"usage","synchronous":"NORMAL","batch_rows":64,"flush_ms":200,"writes":4096,"enqueue_and_commit_ms":ms,"payload_bytes":payload,"wal_before_checkpoint":before,"checkpoint":checkpoint,"checkpoint_us":started.elapsed().as_micros(),"database_bytes":std::fs::metadata(dir.path().join("gateway.sqlite")).unwrap().len()})
    );
    telemetry.shutdown().await.unwrap();
}
