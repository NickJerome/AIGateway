use gateway::bytecode::compile;
use gateway::engine::Invocation;
use gateway::limits::Limits;
use gateway::runtime::clock::SliceClock;
use gateway::runtime::executor::{Executor, ExecutorHandle};
use gateway::runtime::request::{Phase, RequestHandle};
use gateway::worker::Worker;
use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context as TaskContext, Poll};
use std::thread::sleep;
use std::time::{Duration, Instant};

#[test]
fn host_wait_grants_a_fresh_budget() {
    let clock = SliceClock::new(100);
    clock.start_segment();
    sleep(Duration::from_millis(60));
    assert!(!clock.expired());
    // A host wait stops the clock; the next segment gets a fresh 100 ms.
    clock.host_wait();
    assert_eq!(clock.elapsed(), Duration::ZERO);
    clock.start_segment();
    sleep(Duration::from_millis(60));
    assert!(!clock.expired());
    sleep(Duration::from_millis(60));
    assert!(clock.expired());
}

#[test]
fn fairness_yield_keeps_the_accumulated_budget() {
    let clock = SliceClock::new(100);
    clock.start_segment();
    sleep(Duration::from_millis(60));
    clock.yield_pause();
    // The yield paused the segment but kept the accumulated 60 ms.
    assert!(clock.elapsed() >= Duration::from_millis(60));
    assert!(!clock.expired());
    clock.start_segment();
    sleep(Duration::from_millis(60));
    // No fresh budget after a yield: 60 + 60 >= 100 ms.
    assert!(clock.expired());
}

/// A future that parks once until the executor fires its scheduled wakeup.
struct WaitForWakeup {
    handle: ExecutorHandle,
    at: Instant,
    registered: bool,
}

impl Future for WaitForWakeup {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<()> {
        if self.registered {
            return Poll::Ready(());
        }
        self.registered = true;
        self.handle.schedule_wakeup(self.at, cx.waker().clone());
        Poll::Pending
    }
}

#[test]
fn executor_runs_spawned_tasks_to_completion() {
    let mut exec = Executor::new();
    let seen = Rc::new(Cell::new(0));
    for _ in 0..2 {
        let seen = Rc::clone(&seen);
        exec.spawn(async move {
            seen.set(seen.get() + 1);
        });
    }
    assert_eq!(exec.live_tasks(), 2);
    exec.run();
    assert_eq!(exec.live_tasks(), 0);
    assert_eq!(seen.get(), 2);
}

#[test]
fn executor_parks_until_a_scheduled_wakeup() {
    let mut exec = Executor::new();
    let handle = exec.handle();
    let start = Instant::now();
    exec.spawn(WaitForWakeup {
        handle,
        at: start + Duration::from_millis(50),
        registered: false,
    });
    exec.run();
    assert!(start.elapsed() >= Duration::from_millis(50));
    assert_eq!(exec.live_tasks(), 0);
}

fn compile_plugin(source: &str) -> gateway::bytecode::CompiledArtifact {
    compile("plugin.mjs", source).unwrap().code
}

fn invocation() -> Invocation {
    Invocation {
        url: "/".into(),
        method: "GET".into(),
    }
}

fn start(
    worker: &Worker,
    exec: &Executor,
    code: &gateway::bytecode::CompiledArtifact,
) -> RequestHandle {
    let engine = worker.start_request().unwrap();
    let (handle, task) = engine.into_task(&exec.handle(), code, invocation());
    exec.spawn(task.drive());
    handle
}

fn wait_phase(handle: &RequestHandle, phase: Phase) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while handle.phase() != phase {
        assert!(
            Instant::now() < deadline,
            "stuck at {:?}, wanted {phase:?}",
            handle.phase()
        );
        sleep(Duration::from_millis(1));
    }
}

/// Stand-in for the 02 HTTP layer: take the response, then report it sent.
fn http_layer(handle: &RequestHandle) -> String {
    wait_phase(handle, Phase::Responding);
    // D2: observing Responding implies the response slot is written.
    let body = handle
        .take_response()
        .expect("response must be visible once Responding");
    handle.finish_response();
    body
}

#[test]
fn sync_return_reaches_closed_and_returns_permit() {
    let worker = Worker::new();
    let mut exec = Executor::new();
    let code = compile_plugin("export default { fetch(){ return 'ok'; } };");
    let handle = start(&worker, &exec, &code);
    let http = {
        let h = handle.clone();
        std::thread::spawn(move || http_layer(&h))
    };
    exec.run();
    assert_eq!(http.join().unwrap(), "ok");
    assert_eq!(handle.phase(), Phase::Closed);
    assert_eq!(handle.outcome().unwrap().unwrap(), "ok");
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn interleaved_fast_and_slow_requests_recover_independently() {
    let worker = Worker::new();
    let mut exec = Executor::new();
    let fast = compile_plugin(
        "globalThis.state = 'fast-plugin'; export default { async fetch(){ await new Promise(r => setTimeout(r, 10)); state += ':done'; return 'fast:' + state; } };",
    );
    let slow = compile_plugin(
        "globalThis.state = 'slow-plugin'; export default { async fetch(){ await new Promise(r => setTimeout(r, 120)); state += ':done'; return 'slow:' + state; } };",
    );
    let fast_h = start(&worker, &exec, &fast);
    let slow_h = start(&worker, &exec, &slow);
    let (tx, rx) = std::sync::mpsc::channel();
    for (name, h) in [("fast", fast_h), ("slow", slow_h)] {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let body = http_layer(&h);
            tx.send((name, body, Instant::now())).unwrap();
        });
    }
    drop(tx);
    exec.run();
    let mut results: Vec<_> = rx.iter().collect();
    results.sort_by_key(|(_, _, t)| *t);
    // The fast request finishes first; the slow one recovers correctly.
    assert_eq!(results[0].0, "fast");
    // The two engines' global state never mixes.
    assert_eq!(results[0].1, "fast:fast-plugin:done");
    assert_eq!(results[1].1, "slow:slow-plugin:done");
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn io_wait_does_not_consume_the_js_slice() {
    let worker = Worker::with_limits(Limits {
        slice_ms: 100,
        ..Limits::default()
    });
    let mut exec = Executor::new();
    let code = compile_plugin(
        "export default { async fetch(){ await new Promise(r => setTimeout(r, 2000)); return 'slow-io'; } };",
    );
    let handle = start(&worker, &exec, &code);
    let http = {
        let h = handle.clone();
        std::thread::spawn(move || http_layer(&h))
    };
    let started = Instant::now();
    exec.run();
    // A 2 s host I/O wait must not trip the 100 ms JS slice.
    assert!(started.elapsed() >= Duration::from_secs(2));
    assert_eq!(http.join().unwrap(), "slow-io");
    assert_eq!(handle.outcome().unwrap().unwrap(), "slow-io");
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn infinite_loop_is_interrupted() {
    let worker = Worker::with_limits(Limits {
        slice_ms: 100,
        ..Limits::default()
    });
    let mut exec = Executor::new();
    let code = compile_plugin("export default { fetch(){ while (true) {} } };");
    let handle = start(&worker, &exec, &code);
    let started = Instant::now();
    exec.run();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(handle.outcome().unwrap().is_err());
    assert_eq!(handle.phase(), Phase::Closed);
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn infinite_microtask_chain_is_interrupted() {
    let worker = Worker::with_limits(Limits {
        slice_ms: 100,
        ..Limits::default()
    });
    let mut exec = Executor::new();
    let code = compile_plugin(
        "export default { async fetch(){ while (true) { await Promise.resolve(); } } };",
    );
    let handle = start(&worker, &exec, &code);
    let started = Instant::now();
    exec.run();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(handle.outcome().unwrap().is_err());
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn fairness_yield_preserves_the_slice_budget() {
    let worker = Worker::with_limits(Limits {
        slice_ms: 100,
        ..Limits::default()
    });
    let mut exec = Executor::new();
    // Two ~80 ms microtask batches cross many fairness yields; a yield must
    // not hand out a fresh 100 ms budget, so the chain is still interrupted.
    let code = compile_plugin(
        "async function spin(ms){ const t = Date.now(); while (Date.now() - t < ms) { await Promise.resolve(); } }\nexport default { async fetch(){ await spin(80); await spin(80); return 'done'; } };",
    );
    let handle = start(&worker, &exec, &code);
    let started = Instant::now();
    exec.run();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(handle.outcome().unwrap().is_err());
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn cancel_while_waiting_for_host_io() {
    let worker = Worker::new();
    let mut exec = Executor::new();
    let code = compile_plugin(
        "export default { async fetch(){ await new Promise(r => setTimeout(r, 5000)); return 'unreachable'; } };",
    );
    let handle = start(&worker, &exec, &code);
    let canceller = {
        let h = handle.clone();
        std::thread::spawn(move || {
            sleep(Duration::from_millis(100));
            h.cancel();
        })
    };
    let started = Instant::now();
    exec.run();
    canceller.join().unwrap();
    // Cancellation lands within ~100 ms, not after the 5 s timer.
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(handle.outcome().unwrap().is_err());
    assert_eq!(handle.phase(), Phase::Closed);
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn permit_is_returned_exactly_once_on_double_cancel() {
    let worker = Worker::new();
    let mut exec = Executor::new();
    let code = compile_plugin(
        "export default { async fetch(){ await new Promise(r => setTimeout(r, 5000)); return 'unreachable'; } };",
    );
    let handle = start(&worker, &exec, &code);
    handle.cancel();
    // A second cancel is safe: the permit is returned exactly once, never
    // twice (a double return would underflow the worker's in-flight count).
    handle.cancel();
    exec.run();
    assert_eq!(handle.phase(), Phase::Closed);
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn unfinished_execution_is_reported_without_host_tasks() {
    let worker = Worker::new();
    let mut exec = Executor::new();
    // A dangling promise with no host task behind it must not wait forever.
    let code = compile_plugin(
        "export default { async fetch(){ await new Promise(() => {}); return 'unreachable'; } };",
    );
    let handle = start(&worker, &exec, &code);
    let started = Instant::now();
    exec.run();
    assert!(started.elapsed() < Duration::from_secs(1));
    let err = handle.outcome().unwrap().unwrap_err();
    assert!(err.contains("unfinished execution"), "got: {err}");
    assert_eq!(handle.phase(), Phase::Closed);
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn late_host_callback_is_discarded_after_close() {
    let worker = Worker::new();
    let mut exec = Executor::new();
    let code = compile_plugin("export default { fetch(){ return 'ok'; } };");
    let handle = start(&worker, &exec, &code);
    let http = {
        let h = handle.clone();
        std::thread::spawn(move || http_layer(&h))
    };
    exec.run();
    http.join().unwrap();
    assert_eq!(handle.phase(), Phase::Closed);
    // A late host completion after Closed is dropped without touching JS.
    handle.inject_late_fire();
    assert_eq!(handle.late_fire_count(), 1);
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn wait_until_late_registration_is_rejected() {
    let worker = Worker::new();
    let mut exec = Executor::new();
    let code = compile_plugin("export default { fetch(){ return 'ok'; } };");
    let handle = start(&worker, &exec, &code);
    let http = {
        let h = handle.clone();
        std::thread::spawn(move || http_layer(&h))
    };
    exec.run();
    http.join().unwrap();
    // After Closed the registration gate is shut; the JS waitUntil binding
    // shares this gate and throws outside the active phases.
    assert_eq!(handle.phase(), Phase::Closed);
    assert!(!handle.wait_until_open());
}

#[test]
fn wait_until_work_runs_in_the_background_window() {
    let worker = Worker::with_limits(Limits {
        background_ms: 1000,
        ..Limits::default()
    });
    let mut exec = Executor::new();
    // Registered after fetch() returned (a microtask, still an active phase).
    let code = compile_plugin(
        "export default { fetch(req, env, ctx){ queueMicrotask(() => ctx.waitUntil(new Promise(r => setTimeout(r, 60)))); return 'ok'; } };",
    );
    let handle = start(&worker, &exec, &code);
    let http = {
        let h = handle.clone();
        std::thread::spawn(move || http_layer(&h))
    };
    let observed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let observer = {
        let h = handle.clone();
        let observed = std::sync::Arc::clone(&observed);
        std::thread::spawn(move || {
            loop {
                let phase = h.phase();
                {
                    let mut log = observed.lock().unwrap();
                    if log.last() != Some(&phase) {
                        log.push(phase);
                    }
                }
                if phase == Phase::Closed {
                    break;
                }
                sleep(Duration::from_millis(1));
            }
        })
    };
    let started = Instant::now();
    exec.run();
    assert_eq!(http.join().unwrap(), "ok");
    observer.join().unwrap();
    // The Engine and permit stay alive for background work...
    let log = observed.lock().unwrap();
    assert!(log.contains(&Phase::Background), "phases: {log:?}");
    drop(log);
    // ...but the request ends once waitUntil drains, well before the window.
    assert!(started.elapsed() < Duration::from_millis(900));
    assert_eq!(handle.phase(), Phase::Closed);
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn background_window_is_capped() {
    let worker = Worker::with_limits(Limits {
        background_ms: 300,
        ..Limits::default()
    });
    let mut exec = Executor::new();
    // waitUntil on a 10 s timer; the fixed 300 ms window cuts it off.
    let code = compile_plugin(
        "export default { fetch(req, env, ctx){ ctx.waitUntil(new Promise(r => setTimeout(r, 10_000))); return 'ok'; } };",
    );
    let handle = start(&worker, &exec, &code);
    let http = {
        let h = handle.clone();
        std::thread::spawn(move || http_layer(&h))
    };
    let started = Instant::now();
    exec.run();
    assert_eq!(http.join().unwrap(), "ok");
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(300) && elapsed < Duration::from_secs(3),
        "background window must cap at 300 ms, took {elapsed:?}"
    );
    assert_eq!(handle.phase(), Phase::Closed);
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn poke_before_the_first_suspend_registration_is_never_lost() {
    // finish variant: the poke lands after the first readiness check but
    // before the waker registration; only the post-registration re-check
    // saves it. Without the re-check this test hangs forever.
    {
        let worker = Worker::new();
        let mut exec = Executor::new();
        let code = compile_plugin("export default { fetch(){ return 'ok'; } };");
        let handle = start(&worker, &exec, &code);
        handle.poke_before_next_suspend(|h| h.finish_response());
        exec.run();
        assert_eq!(handle.phase(), Phase::Closed);
        assert_eq!(handle.outcome().unwrap().unwrap(), "ok");
        assert_eq!(worker.in_flight(), 0);
    }
    // cancel variant: same injection point, opposite outcome.
    {
        let worker = Worker::new();
        let mut exec = Executor::new();
        let code = compile_plugin("export default { fetch(){ return 'ok'; } };");
        let handle = start(&worker, &exec, &code);
        handle.poke_before_next_suspend(|h| h.cancel());
        exec.run();
        assert_eq!(handle.phase(), Phase::Closed);
        assert!(handle.outcome().unwrap().is_err());
        assert_eq!(worker.in_flight(), 0);
    }
}

#[test]
fn response_instance_status_and_headers_reach_the_slot() {
    let worker = Worker::new();
    let mut exec = Executor::new();
    let code = compile_plugin(
        "export default { fetch(){ return new Response('created', { status: 201, headers: { 'X-A': '1', 'X-B': '2' } }); } };",
    );
    let handle = start(&worker, &exec, &code);
    let http = {
        let h = handle.clone();
        std::thread::spawn(move || {
            wait_phase(&h, Phase::Responding);
            // The slot carries the full response: status, headers, body.
            assert_eq!(h.response_status(), Some(201));
            assert_eq!(
                h.response_headers().unwrap(),
                vec![
                    ("x-a".to_string(), "1".to_string()),
                    ("x-b".to_string(), "2".to_string()),
                    (
                        "content-type".to_string(),
                        "text/plain;charset=UTF-8".to_string()
                    )
                ]
            );
            let body = h.take_response().expect("response visible once Responding");
            h.finish_response();
            body
        })
    };
    exec.run();
    assert_eq!(http.join().unwrap(), "created");
    assert_eq!(handle.outcome().unwrap().unwrap(), "created");
    // Response returned, Engine/permit lifecycle unchanged.
    assert_eq!(handle.phase(), Phase::Closed);
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn string_responses_keep_legacy_slot_semantics() {
    let worker = Worker::new();
    let mut exec = Executor::new();
    let code = compile_plugin("export default { fetch(){ return 'plain'; } };");
    let handle = start(&worker, &exec, &code);
    let http = {
        let h = handle.clone();
        std::thread::spawn(move || {
            wait_phase(&h, Phase::Responding);
            // The M2 stub semantics adapt: a string is a 200 with no headers.
            assert_eq!(h.response_status(), Some(200));
            assert_eq!(
                h.response_headers().unwrap(),
                Vec::<(String, String)>::new()
            );
            let body = h.take_response().expect("response visible once Responding");
            h.finish_response();
            body
        })
    };
    exec.run();
    assert_eq!(http.join().unwrap(), "plain");
    assert_eq!(handle.outcome().unwrap().unwrap(), "plain");
    assert_eq!(handle.phase(), Phase::Closed);
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn async_response_instance_reaches_the_slot_after_a_timer() {
    let worker = Worker::new();
    let mut exec = Executor::new();
    let code = compile_plugin(
        "export default { async fetch(){ await new Promise(r => setTimeout(r, 30)); return Response.json({ ok: true }); } };",
    );
    let handle = start(&worker, &exec, &code);
    let http = {
        let h = handle.clone();
        std::thread::spawn(move || {
            wait_phase(&h, Phase::Responding);
            assert_eq!(h.response_status(), Some(200));
            assert_eq!(
                h.response_headers().unwrap(),
                vec![("content-type".to_string(), "application/json".to_string())]
            );
            let body = h.take_response().expect("response visible once Responding");
            h.finish_response();
            body
        })
    };
    exec.run();
    assert_eq!(http.join().unwrap(), "{\"ok\":true}");
    assert_eq!(handle.phase(), Phase::Closed);
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn interval_does_not_extend_the_request_lifetime() {
    let worker = Worker::new();
    let mut exec = Executor::new();
    // A dangling promise backed only by an interval: intervals never count
    // as host work that keeps the request alive (spec 01 line 32).
    let code = compile_plugin(
        "export default { async fetch(){ setInterval(() => {}, 10); await new Promise(() => {}); return 'unreachable'; } };",
    );
    let handle = start(&worker, &exec, &code);
    let started = Instant::now();
    exec.run();
    assert!(started.elapsed() < Duration::from_secs(1));
    let err = handle.outcome().unwrap().unwrap_err();
    assert!(err.contains("unfinished execution"), "got: {err}");
    assert_eq!(handle.phase(), Phase::Closed);
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn interval_callback_errors_enter_request_error_handling() {
    let worker = Worker::new();
    let mut exec = Executor::new();
    let code = compile_plugin(
        "export default { async fetch(){ setInterval(() => { throw new Error('interval boom'); }, 10); await new Promise(r => setTimeout(r, 5000)); return 'unreachable'; } };",
    );
    let handle = start(&worker, &exec, &code);
    let started = Instant::now();
    exec.run();
    // The interval exception aborts the request long before the 5 s timer.
    assert!(started.elapsed() < Duration::from_secs(2));
    let err = handle.outcome().unwrap().unwrap_err();
    assert!(err.contains("interval boom"), "got: {err}");
    assert_eq!(handle.phase(), Phase::Closed);
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn consumed_response_body_extracts_as_empty_stub_semantics() {
    let worker = Worker::new();
    let mut exec = Executor::new();
    // Stub semantics until 02 streams: a Response whose body was already read
    // still hands its status and headers to the slot; the body extracts as
    // empty (the bytes were consumed by text()).
    let code = compile_plugin(
        "export default { async fetch(){ const r = new Response('hi', { status: 202, headers: { 'X-A': '1' } }); const t = await r.text(); if (t !== 'hi' || !r.bodyUsed) { throw new Error('precondition'); } return r; } };",
    );
    let handle = start(&worker, &exec, &code);
    let http = {
        let h = handle.clone();
        std::thread::spawn(move || {
            wait_phase(&h, Phase::Responding);
            assert_eq!(h.response_status(), Some(202));
            assert_eq!(
                h.response_headers().unwrap(),
                vec![
                    ("x-a".to_string(), "1".to_string()),
                    (
                        "content-type".to_string(),
                        "text/plain;charset=UTF-8".to_string()
                    )
                ]
            );
            let body = h.take_response().expect("response visible once Responding");
            h.finish_response();
            body
        })
    };
    exec.run();
    assert_eq!(http.join().unwrap(), "");
    assert_eq!(handle.phase(), Phase::Closed);
    assert_eq!(worker.in_flight(), 0);
}
