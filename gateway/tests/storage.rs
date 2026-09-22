use futures::executor::block_on;
use gateway::storage::Database;
#[test]
fn migrations_wal_full_and_future_schema() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 2, 2).unwrap();
    block_on(db.run(|c| {
        assert_eq!(
            c.pragma_query_value(None, "journal_mode", |r| r.get::<_, String>(0))?,
            "wal"
        );
        assert_eq!(
            c.pragma_query_value(None, "synchronous", |r| r.get::<_, i64>(0))?,
            2
        );
        c.pragma_update(None, "user_version", 99)?;
        Ok(())
    }))
    .unwrap();
    drop(db);
    assert!(Database::open(dir.path(), 1, 2).is_err());
}
#[test]
fn migration_failure_rolls_back_without_erasing_data() {
    let dir = tempfile::tempdir().unwrap();
    let conn = rusqlite::Connection::open(dir.path().join("gateway.sqlite")).unwrap();
    conn.execute_batch(
        "CREATE TABLE plugin_settings (value TEXT); INSERT INTO plugin_settings VALUES ('keep')",
    )
    .unwrap();
    assert!(Database::open(dir.path(), 1, 2).is_err());
    assert_eq!(
        conn.query_row("SELECT value FROM plugin_settings", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "keep"
    );
    assert_eq!(
        conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn cancelled_queued_jobs_do_not_run_started_jobs_finish() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 1, 2).unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let committed = Arc::new(AtomicBool::new(false));
    let flag = committed.clone();
    let first = db.run(move |_| {
        entered_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        flag.store(true, Ordering::SeqCst);
        Ok(())
    });
    let mut first = Box::pin(first);
    let w = futures::task::noop_waker();
    let mut cx = std::task::Context::from_waker(&w);
    assert!(std::future::Future::poll(first.as_mut(), &mut cx).is_pending());
    entered_rx.recv().unwrap();
    let second = db.run(|_| -> anyhow::Result<()> { panic!("cancelled job executed") });
    let mut second = Box::pin(second);
    assert!(std::future::Future::poll(second.as_mut(), &mut cx).is_pending());
    drop(second);
    drop(first);
    release_tx.send(()).unwrap();
    block_on(db.run(|_| Ok(()))).unwrap();
    assert!(committed.load(Ordering::SeqCst));
}
#[test]
fn concurrent_full_commits_and_disk_full_are_not_silent() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 4, 8).unwrap();
    let timings = block_on(async {
        let kv = db.namespace("concurrent").await.unwrap();
        let jobs = (0..32).map(|i| {
            let kv = kv.clone();
            async move {
                let start = std::time::Instant::now();
                kv.put(format!("key-{i}"), vec![i as u8; 1024], None)
                    .await
                    .unwrap();
                start.elapsed().as_micros()
            }
        });
        let times = futures::future::join_all(jobs).await;
        assert_eq!(kv.list("key-".into(), 100).await.unwrap().keys.len(), 32);
        times
    });
    let mut timings = timings;
    timings.sort();
    eprintln!(
        "FULL commit, 4 workers, 32 queued writes: median={}us max={}us",
        timings[16], timings[31]
    );
    drop(db);
    let db = Database::open(dir.path(), 1, 8).unwrap();
    block_on(async {
        let kv = db.namespace("full").await.unwrap();
        db.run(|c| {
            let pages = c.pragma_query_value(None, "page_count", |r| r.get::<_, i64>(0))?;
            c.pragma_update(None, "max_page_count", pages + 1)?;
            Ok(())
        })
        .await
        .unwrap();
        assert!(
            kv.put("too-large".into(), vec![42; 1024 * 1024], None)
                .await
                .is_err()
        );
        assert!(kv.get("too-large".into()).await.unwrap().is_none());
    });
}
#[test]
fn lock_wait_is_bounded_and_failure_visible() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 1, 2).unwrap();
    let kv = block_on(db.namespace("locked")).unwrap();
    let external = rusqlite::Connection::open(dir.path().join("gateway.sqlite")).unwrap();
    external.execute_batch("BEGIN IMMEDIATE").unwrap();
    let start = std::time::Instant::now();
    assert!(block_on(kv.put("x".into(), vec![1], None)).is_err());
    assert!(start.elapsed() < std::time::Duration::from_secs(7));
    external.execute_batch("ROLLBACK").unwrap();
    block_on(kv.put("x".into(), vec![1], None)).unwrap();
}

#[test]
fn upgrades_each_existing_schema_without_losing_plugin_settings() {
    let migrations = [
        include_str!("../src/storage/001.sql"),
        include_str!("../src/storage/002.sql"),
        include_str!("../src/storage/003.sql"),
        include_str!("../src/storage/004.sql"),
    ];
    for version in 1..=4 {
        let dir = tempfile::tempdir().unwrap();
        let conn = rusqlite::Connection::open(dir.path().join("gateway.sqlite")).unwrap();
        for sql in &migrations[..version] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", version as i64)
            .unwrap();
        conn.execute(
            "INSERT INTO plugin_settings VALUES ('kept','disabled',0,'manifest',NULL,'{}',1)",
            [],
        )
        .unwrap();
        drop(conn);
        let db = Database::open(dir.path(), 1, 8).unwrap();
        block_on(db.run(|c| {
            assert_eq!(
                c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?,
                4
            );
            assert_eq!(
                c.query_row(
                    "SELECT manifest FROM plugin_settings WHERE name='kept'",
                    [],
                    |r| r.get::<_, String>(0)
                )?,
                "manifest"
            );
            c.prepare("SELECT token_hash FROM identity_sessions")?;
            Ok(())
        }))
        .unwrap();
    }
}
