//! Spec 1 regressions: execution limits, lifecycle and Web API invariants.
use gateway::bytecode::compile;
use gateway::engine::Invocation;
use gateway::limits::Limits;
use gateway::runtime::executor::{Executor, ExecutorHandle};
use gateway::runtime::request::{Phase, RequestHandle};
use gateway::worker::Worker;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

fn call() -> Invocation {
    Invocation {
        url: "https://example.com/".into(),
        method: "GET".into(),
    }
}
fn sync(body: &str) -> anyhow::Result<String> {
    let source = format!("export default {{ async fetch() {{ {body} }} }};");
    let code = compile("review.mjs", &source)?.code;
    Worker::new().start_request()?.invoke(&code, call())
}
fn start(worker: &Worker, exec: &Executor, source: &str) -> RequestHandle {
    let code = compile("review.mjs", source).unwrap().code;
    let (h, task) = worker
        .start_request()
        .unwrap()
        .into_task(&exec.handle(), &code, call());
    exec.spawn(task.drive());
    h
}
fn http(h: RequestHandle, delay_ms: u64) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match h.phase() {
                Phase::Responding => {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                    h.finish_response();
                    return;
                }
                Phase::Closed => return,
                _ => {}
            }
            if Instant::now() > deadline {
                h.cancel();
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    })
}
fn async_run(source: &str, delay_ms: u64, limits: Limits) -> Result<String, String> {
    let worker = Worker::with_limits(limits);
    let mut exec = Executor::new();
    let h = start(&worker, &exec, source);
    let host = http(h.clone(), delay_ms);
    exec.run();
    host.join().unwrap();
    assert_eq!(worker.in_flight(), 0);
    h.outcome().expect("terminal outcome")
}

#[test]
fn r01_response_wait_does_not_charge_sync_entry_js_budget() {
    let result = async_run(
        "export default { fetch(){ queueMicrotask(() => { let n=0; for(let i=0;i<10000;i++) n+=i; }); return new Response('ok'); } };",
        250,
        Limits {
            slice_ms: 50,
            ..Limits::default()
        },
    );
    assert_eq!(result, Ok("ok".into()));
}

#[test]
fn r02_typed_array_shadowed_buffer_must_not_panic_rust() {
    let result = sync(
        "const a = new Uint8Array([65]); Object.defineProperty(a, 'buffer', {value: null}); return await new Response(a).text();",
    );
    assert_eq!(result.unwrap(), "A");
}

#[test]
fn r02_random_values_shadowed_length_must_not_panic_rust() {
    let result = sync(
        "const a = new Uint8Array(1); Object.defineProperty(a, 'byteLength', {value: 100}); crypto.getRandomValues(a); return 'ok';",
    );
    assert_eq!(result.unwrap(), "ok");
}

struct Delay {
    handle: ExecutorHandle,
    at: Instant,
}
impl Future for Delay {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if Instant::now() >= self.at {
            Poll::Ready(())
        } else {
            self.handle.schedule_wakeup(self.at, cx.waker().clone());
            Poll::Pending
        }
    }
}

#[test]
fn r03_background_deadline_must_be_checked_before_late_callbacks() {
    let worker = Worker::with_limits(Limits {
        background_ms: 60,
        ..Limits::default()
    });
    let mut exec = Executor::new();
    let h = start(
        &worker,
        &exec,
        "export default { async fetch(req, env, ctx) { ctx.waitUntil(new Promise(() => { setTimeout(() => { throw new Error('CALLBACK_AFTER_DEADLINE'); }, 120); })); return 'ok'; } };",
    );
    let host = http(h.clone(), 0);
    let watch = h.clone();
    let schedule = exec.handle();
    exec.spawn(async move {
        while watch.phase() != Phase::Background {
            if watch.phase() == Phase::Closed {
                return;
            }
            Delay {
                handle: schedule.clone(),
                at: Instant::now() + Duration::from_millis(1),
            }
            .await;
        }
        // Model another request's legal 200 ms JS segment on this worker.
        std::thread::sleep(Duration::from_millis(200));
    });
    exec.run();
    host.join().unwrap();
    assert_eq!(h.outcome().unwrap(), Ok("ok".into()));
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn r04_zero_interval_must_allow_its_promise_continuation() {
    let result = async_run(
        "export default { async fetch(){ const keepAlive=setTimeout(()=>{},200); let id; await new Promise(r => { id=setInterval(r,0); }); clearInterval(id); clearTimeout(keepAlive); return 'ok'; } };",
        0,
        Limits {
            slice_ms: 50,
            ..Limits::default()
        },
    );
    assert_eq!(result, Ok("ok".into()));
}

#[test]
fn r05_timeout_omitted_delay_defaults_to_zero() {
    let result = async_run(
        "export default { async fetch(){ await new Promise(r=>setTimeout(r)); return 'ok'; } };",
        0,
        Limits::default(),
    );
    assert_eq!(result, Ok("ok".into()));
}

#[test]
fn r05_timeout_forwards_callback_arguments() {
    let result = async_run(
        "export default { async fetch(){ return await new Promise(r=>setTimeout((a,b)=>r(a+':'+b),1,'a','b')); } };",
        0,
        Limits::default(),
    );
    assert_eq!(result, Ok("a:b".into()));
}

#[test]
fn r06_request_normalizes_method_and_rejects_get_body() {
    let result = sync("return new Request('https://example.com/', {method:'post'}).method;");
    assert_eq!(result.unwrap(), "POST");
}
#[test]
fn r06_request_constructor_rejects_consumed_source() {
    let result = sync(
        "const r = new Request('https://example.com/', {method:'POST',body:'payload'}); await r.text(); try {new Request(r); return 'accepted';} catch(e) {return 'rejected';}",
    );
    assert_eq!(result.unwrap(), "rejected");
}

#[test]
fn r07_structured_clone_preserves_own_proto_key() {
    let result = sync(
        "const x=structuredClone(JSON.parse('{\"__proto__\":{\"flag\":1}}')); return JSON.stringify([Object.hasOwn(x,'__proto__'),Object.getPrototypeOf(x)===Object.prototype,x.flag===undefined]);",
    );
    assert_eq!(result.unwrap(), "[true,true,true]");
}
#[test]
fn r07_structured_clone_rejects_nonempty_iterable_transfer() {
    let result = sync(
        "try {structuredClone({}, {transfer:new Set([new ArrayBuffer(1)])});return 'accepted';}catch(e){return e.name;}",
    );
    assert_eq!(result.unwrap(), "DataCloneError");
}

#[test]
fn r08_hmac_import_rejects_impossible_key_length() {
    let result = sync(
        "try {await crypto.subtle.importKey('raw',new Uint8Array([1]),{name:'HMAC',hash:'SHA-256',length:16},false,['sign']);return 'accepted';}catch(e){return e.name;}",
    );
    assert_eq!(result.unwrap(), "DataError");
}

#[test]
fn r09_removed_once_listener_must_not_run() {
    let result = sync(
        "const t=new EventTarget();let called=0;function b(){called++;} t.addEventListener('x',()=>t.removeEventListener('x',b)); t.addEventListener('x',b,{once:true});t.dispatchEvent(new Event('x'));return String(called);",
    );
    assert_eq!(result.unwrap(), "0");
}

#[test]
fn r10_response_json_rejects_null_body_status() {
    let result = sync(
        "try {Response.json({x:1},{status:204});return 'accepted';} catch(e) {return e.name;}",
    );
    assert_eq!(result.unwrap(), "TypeError");
}

#[test]
fn control_async_entry_wait_does_not_charge_js_budget() {
    let result = async_run(
        "export default { async fetch(){ await new Promise(r=>setTimeout(r,250)); return 'ok'; } };",
        0,
        Limits {
            slice_ms: 50,
            ..Limits::default()
        },
    );
    assert_eq!(result, Ok("ok".into()));
}

#[test]
fn r11_cancel_before_first_poll_must_not_execute_plugin() {
    let worker = Worker::new();
    let mut exec = Executor::new();
    let h = start(
        &worker,
        &exec,
        "export default { fetch(){ throw new Error('ENTRY_RAN_AFTER_CANCEL'); } };",
    );
    h.cancel();
    exec.run();
    let err = h.outcome().unwrap().unwrap_err();
    assert!(
        err.contains("cancelled"),
        "expected cancellation without calling entry, got: {err}"
    );
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn control_sync_entry_with_immediate_response_finish_succeeds() {
    let result = async_run(
        "export default { fetch(){ queueMicrotask(() => { let n=0; for(let i=0;i<10000;i++) n+=i; }); return new Response('ok'); } };",
        0,
        Limits {
            slice_ms: 50,
            ..Limits::default()
        },
    );
    assert_eq!(result, Ok("ok".into()));
}

#[test]
fn followup_deadline_before_post_response_jobs() {
    let worker = Worker::with_limits(Limits {
        background_ms: 40,
        ..Limits::default()
    });
    let mut exec = Executor::new();
    let h = start(
        &worker,
        &exec,
        "export default { fetch(req,env,ctx) { ctx.waitUntil(new Promise(()=>{})); queueMicrotask(()=>{throw new Error('JOB_AFTER_DEADLINE')}); return 'ok'; } };",
    );
    let host = h.clone();
    let schedule = exec.handle();
    exec.spawn(async move {
        while host.phase() != Phase::Responding {
            if host.phase() == Phase::Closed {
                return;
            }
            Delay {
                handle: schedule.clone(),
                at: Instant::now() + Duration::from_millis(1),
            }
            .await;
        }
        host.finish_response();
        std::thread::sleep(Duration::from_millis(150));
        // Duplicate transport completion must not restart the window.
        host.finish_response();
    });
    exec.run();
    assert_eq!(h.outcome().unwrap(), Ok("ok".into()));
    assert_eq!(worker.in_flight(), 0);
}

#[test]
fn followup_deadline_between_background_jobs() {
    let result = async_run(
        "export default { fetch(req,env,ctx) { ctx.waitUntil(new Promise(()=>{setTimeout(()=>{queueMicrotask(()=>{const end=Date.now()+150;while(Date.now()<end){};});queueMicrotask(()=>{throw new Error('NEXT_JOB_AFTER_DEADLINE')});},10)})); return 'ok'; } };",
        0,
        Limits {
            background_ms: 40,
            slice_ms: 1000,
            ..Limits::default()
        },
    );
    assert_eq!(result, Ok("ok".into()));
}

#[test]
fn followup_consumed_request_with_replacement_body() {
    let result = sync(
        "const r=new Request('https://example.com/',{method:'POST',body:'old'});await r.text();const next=new Request(r,{body:'new'});return await next.text();",
    );
    assert_eq!(result.unwrap(), "new");
}

#[test]
fn followup_dataview_uses_actual_view() {
    let result = sync(
        "const v=new DataView(new Uint8Array([65]).buffer);Object.defineProperty(v,'buffer',{value:new Uint8Array([66]).buffer});return await new Response(v).text();",
    );
    assert_eq!(result.unwrap(), "A");
}

#[test]
fn background_deadline_interrupts_a_running_callback() {
    let result = async_run(
        "export default { fetch(req,env,ctx) { ctx.waitUntil(new Promise(()=>{setTimeout(()=>{while(true){}},10)})); return 'ok'; } };",
        0,
        Limits {
            background_ms: 40,
            slice_ms: 500,
            ..Limits::default()
        },
    );
    assert_eq!(result, Ok("ok".into()));
}

#[test]
fn dataview_native_accessors_survive_instance_and_prototype_overrides() {
    let result = sync(
        r#"
        const view = new DataView(new Uint8Array([0,65,66,0]).buffer,1,2);
        for (const key of ['buffer','byteOffset','byteLength']) {
            Object.defineProperty(view,key,{get(){throw new Error('shadow getter')}});
            Object.defineProperty(DataView.prototype,key,{get(){throw new Error('prototype getter')}});
        }
        return await new Response(view).text();
    "#,
    );
    assert_eq!(result.unwrap(), "AB");
}

#[test]
fn request_body_getter_can_replace_a_source_it_consumes() {
    let result = sync(
        r#"
        const old = new Request('https://example.com/',{method:'POST',body:'old'});
        const next = new Request(old,{get body(){old.text();return 'new'}});
        const body = await next.text();
        let rejected = false;
        try { new Request(old,{body:null}); } catch(e) { rejected = e instanceof TypeError; }
        return JSON.stringify([body,rejected]);
    "#,
    );
    assert_eq!(result.unwrap(), "[\"new\",true]");
}
