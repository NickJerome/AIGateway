use futures::executor::block_on;
use gateway::storage::Database;
#[test]
fn namespace_isolation_ttl_overwrite_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 2, 8).unwrap();
    block_on(async {
        let a = db.namespace("stable-id").await.unwrap();
        let b = db.namespace("other-id").await.unwrap();
        a.put_at("key".into(), b"a".to_vec(), Some(60), 100)
            .await
            .unwrap();
        b.put("key".into(), b"b".to_vec(), None).await.unwrap();
        assert_eq!(
            a.get_at("key".into(), 159).await.unwrap(),
            Some(b"a".to_vec())
        );
        assert_eq!(a.get_at("key".into(), 160).await.unwrap(), None);
        a.put_at("key".into(), b"new".to_vec(), None, 200)
            .await
            .unwrap();
        assert_eq!(
            a.get_at("key".into(), 999).await.unwrap(),
            Some(b"new".to_vec())
        );
        assert_eq!(b.get("key".into()).await.unwrap(), Some(b"b".to_vec()));
    });
    drop(db);
    let db = Database::open(dir.path(), 1, 8).unwrap();
    block_on(async {
        let kv = db.namespace("stable-id").await.unwrap();
        assert_eq!(kv.get("key".into()).await.unwrap(), Some(b"new".to_vec()));
    });
}
#[test]
fn literal_prefix_boundaries_and_idempotent_delete() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 1, 8).unwrap();
    block_on(async {
        let kv = db.namespace("x'; DROP TABLE resources;--").await.unwrap();
        for key in ["a%1", "a%2", "abc", "a_1"] {
            kv.put(key.into(), vec![0, 255], None).await.unwrap();
        }
        let listed = kv.list("a%".into(), 1).await.unwrap();
        assert_eq!(listed.keys[0].name, "a%1");
        assert!(!listed.list_complete);
        assert!(kv.list("a%".into(), 2).await.unwrap().list_complete);
        assert!(kv.put("x".into(), vec![], Some(59)).await.is_err());
        assert!(kv.put("x".repeat(513), vec![], None).await.is_err());
        assert!(kv.list("".into(), 1001).await.is_err());
        kv.delete("absent".into()).await.unwrap();
        kv.delete("a%1".into()).await.unwrap();
        assert!(kv.get("a%1".into()).await.unwrap().is_none());
    });
}
#[test]
fn expiration_uses_execution_time_after_queue_wait() {
    use std::sync::mpsc;
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(dir.path(), 1, 4).unwrap();
    let kv = block_on(db.namespace("clock")).unwrap();
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    };
    block_on(kv.put_at("expiring".into(), vec![1], Some(60), now() - 59)).unwrap();
    let (entered, ready) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let mut blocker = Box::pin(db.run(move |_| {
        entered.send(()).unwrap();
        wait.recv().unwrap();
        Ok(())
    }));
    let w = futures::task::noop_waker();
    let mut cx = std::task::Context::from_waker(&w);
    assert!(std::future::Future::poll(blocker.as_mut(), &mut cx).is_pending());
    ready.recv().unwrap();
    let mut get = Box::pin(kv.get("expiring".into()));
    let mut put = Box::pin(kv.put("written".into(), vec![1], Some(60)));
    assert!(std::future::Future::poll(get.as_mut(), &mut cx).is_pending());
    assert!(std::future::Future::poll(put.as_mut(), &mut cx).is_pending());
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let executed = now();
    release.send(()).unwrap();
    block_on(blocker).unwrap();
    assert!(block_on(get).unwrap().is_none());
    block_on(put).unwrap();
    let expiration = block_on(kv.list("written".into(), 1)).unwrap().keys[0]
        .expiration
        .unwrap();
    assert!(expiration >= executed + 60);
    block_on(kv.put("a\0b".into(), vec![2], None)).unwrap();
    assert_eq!(
        block_on(kv.list("a\0".into(), 1)).unwrap().keys[0].name,
        "a\0b"
    );
}
