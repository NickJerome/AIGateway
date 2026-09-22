use gateway::limits::Limits;
use gateway::worker::Worker;

#[test]
fn concurrency_limit_is_shared_by_request_engines() {
    let worker = Worker::with_limits(Limits {
        max_in_flight: 2,
        ..Limits::default()
    });
    let a = worker.start_request().unwrap();
    let b = worker.start_request().unwrap();
    assert_eq!(worker.in_flight(), 2);
    assert!(worker.start_request().is_err());
    drop(a);
    assert_eq!(worker.in_flight(), 1);
    let c = worker.start_request().unwrap();
    drop((b, c));
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn failed_context_creation_returns_the_permit() {
    let worker = Worker::with_limits(Limits {
        memory_bytes: 1024,
        max_in_flight: 1,
        ..Limits::default()
    });
    for _ in 0..3 {
        assert!(worker.start_request().is_err());
        assert_eq!(worker.in_flight(), 0);
    }
}

#[test]
fn workers_have_independent_admission_limits() {
    let limits = Limits {
        max_in_flight: 1,
        ..Limits::default()
    };
    let a = Worker::with_limits(limits);
    let b = Worker::with_limits(limits);
    let _a_request = a.start_request().unwrap();
    assert!(a.start_request().is_err());
    let _b_request = b.start_request().unwrap();
    assert_eq!(a.in_flight(), 1);
    assert_eq!(b.in_flight(), 1);
}
