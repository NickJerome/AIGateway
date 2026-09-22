//! Request lifecycle task: the state machine, minimal host bindings, and the
//! suspend/wake protocol between the owning worker thread and the HTTP layer.

use crate::engine::{Engine, Invocation, call_entry, describe, stringify};
use crate::runtime::executor::ExecutorHandle;
use anyhow::{Result, anyhow};
use rquickjs::Ctx;
use rquickjs::function::{Args, Rest};
use rquickjs::promise::PromiseState;
use rquickjs::{Class, Coerced, FromJs, Function, Object, Persistent, Promise, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context as TaskContext, Poll};
use std::time::{Duration, Instant};

/// Normal termination when the fixed background window ends.
#[derive(Debug)]
struct BackgroundExpired;
impl std::fmt::Display for BackgroundExpired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("background deadline reached")
    }
}
impl std::error::Error for BackgroundExpired {}

/// Request lifecycle phases, advanced only by the owning request task.
/// Entry errors, cancellation and timeouts move from any active phase to
/// Closing; teardown runs exactly once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Created,
    Running,
    Responding,
    Background,
    Closing,
    Closed,
}

/// Test hook payload for the D1 regression: a poke injected mid-`poll`.
type PreSuspendPoke = Box<dyn FnOnce(&RequestHandle) + Send>;

/// The slotted response for the HTTP layer: owned data only, never a JS
/// value (spec 01: cross-thread messages carry owned data and IDs).
pub struct HostResponse {
    pub web_socket: Option<Arc<crate::websocket::Socket>>,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub stream: Option<Arc<crate::network::NativeBody>>,
}

/// Cross-thread state shared between the request task and the HTTP layer.
/// Carries owned data, IDs and cancellable host results only, never JS values.
struct Shared {
    js_memory: AtomicUsize,
    host_budget: Mutex<Option<crate::network::Budget>>,
    phase: Mutex<Phase>,
    closed: async_channel::Sender<()>,
    closed_rx: async_channel::Receiver<()>,
    response: Mutex<Option<HostResponse>>,
    outcome: Mutex<Option<Result<String, String>>>,
    cancel: Arc<AtomicBool>,
    io_ready: AtomicBool,
    response_ready: async_channel::Sender<()>,
    response_rx: async_channel::Receiver<()>,
    response_finished_at: Mutex<Option<Instant>>,
    deadline: Arc<std::sync::OnceLock<Instant>>,
    background_ms: u64,
    task_waker: Mutex<Option<std::task::Waker>>,
    late_fires: AtomicUsize,
    pre_suspend_poke: Mutex<Option<PreSuspendPoke>>,
}

impl Shared {
    fn new(
        cancel: Arc<AtomicBool>,
        deadline: Arc<std::sync::OnceLock<Instant>>,
        background_ms: u64,
    ) -> Arc<Shared> {
        let (closed, closed_rx) = async_channel::bounded(1);
        let (response_ready, response_rx) = async_channel::bounded(1);
        Arc::new(Shared {
            js_memory: AtomicUsize::new(0),
            host_budget: Mutex::new(None),
            closed,
            closed_rx,
            response_ready,
            response_rx,
            deadline,
            background_ms,
            io_ready: AtomicBool::new(false),
            phase: Mutex::new(Phase::Created),
            response: Mutex::new(None),
            outcome: Mutex::new(None),
            cancel,
            response_finished_at: Mutex::new(None),
            task_waker: Mutex::new(None),
            late_fires: AtomicUsize::new(0),
            pre_suspend_poke: Mutex::new(None),
        })
    }
}

/// Wake the request task. Dekker's protocol, side A: the state store (SeqCst
/// or under the `phase` lock) happens before the waker is read under the
/// `task_waker` lock; side B in `Suspend::poll` registers the waker first and
/// then re-checks the state, so a wakeup can never slip between the two sides.
fn poke(shared: &Shared) {
    let waker = shared.task_waker.lock().unwrap().clone();
    if let Some(waker) = waker {
        waker.wake();
    }
}

/// Send handle for the HTTP layer (02) and tests. Carries no JS values.
#[derive(Clone)]
pub struct RequestHandle {
    shared: Arc<Shared>,
}

impl RequestHandle {
    pub(crate) fn memory(&self) -> (usize, usize) {
        (
            self.shared.js_memory.load(Ordering::Relaxed),
            self.shared
                .host_budget
                .lock()
                .unwrap()
                .as_ref()
                .map_or(0, crate::network::Budget::used),
        )
    }
    pub(crate) fn io_ready(&self) {
        self.shared.io_ready.store(true, Ordering::SeqCst);
        poke(&self.shared);
    }
    pub(crate) fn clear_io_ready(&self) {
        self.shared.io_ready.store(false, Ordering::SeqCst);
    }
    pub async fn response(&self) -> Option<HostResponse> {
        let _ = self.shared.response_rx.recv().await;
        self.shared.response.lock().unwrap().take()
    }
    pub async fn closed(&self) {
        let _ = self.shared.closed_rx.recv().await;
    }

    pub fn phase(&self) -> Phase {
        *self.shared.phase.lock().unwrap()
    }

    /// Cancellation: enters Closing from any active phase, exactly once.
    pub fn cancel(&self) {
        // Store first, then read the waker under the lock (Dekker side A).
        self.shared.cancel.store(true, Ordering::SeqCst);
        poke(&self.shared);
    }

    /// 02 stub: take the finished response body once the phase is Responding.
    pub fn take_response(&self) -> Option<String> {
        let response = self.shared.response.lock().unwrap().take()?;
        Some(String::from_utf8_lossy(&response.body).into_owned())
    }

    /// Peek the slotted response status; does not consume the body.
    pub fn response_status(&self) -> Option<u16> {
        self.shared
            .response
            .lock()
            .unwrap()
            .as_ref()
            .map(|r| r.status)
    }

    /// Peek the slotted response headers (insertion order, lowercased names).
    pub fn response_headers(&self) -> Option<Vec<(String, String)>> {
        self.shared
            .response
            .lock()
            .unwrap()
            .as_ref()
            .map(|r| r.headers.clone())
    }

    /// 02 stub: the response has been fully sent; background work may start.
    /// Never releases the Engine or the permit.
    pub fn finish_response(&self) {
        let at = *self
            .shared
            .response_finished_at
            .lock()
            .unwrap()
            .get_or_insert_with(Instant::now);
        let _ = self
            .shared
            .deadline
            .set(at + Duration::from_millis(self.shared.background_ms));
        poke(&self.shared);
    }

    /// Terminal outcome, available once the phase is Closed.
    pub fn outcome(&self) -> Option<Result<String, String>> {
        self.shared.outcome.lock().unwrap().clone()
    }

    /// Simulate a late host completion arriving after teardown.
    #[doc(hidden)]
    pub fn inject_late_fire(&self) {
        if self.phase() == Phase::Closed {
            // Late callbacks check Closed and are dropped; they never touch JS.
            self.shared.late_fires.fetch_add(1, Ordering::SeqCst);
            return;
        }
        poke(&self.shared);
    }

    #[doc(hidden)]
    pub fn late_fire_count(&self) -> usize {
        self.shared.late_fires.load(Ordering::SeqCst)
    }

    /// Whether `ctx.waitUntil` registration is still open. The JS binding
    /// shares this gate: outside the active phases it throws.
    #[doc(hidden)]
    pub fn wait_until_open(&self) -> bool {
        wait_until_open(self.phase())
    }

    /// Test hook: run `f` inside the next `Suspend::poll`, after the first
    /// readiness check but before the waker is registered (D1 regression).
    #[doc(hidden)]
    pub fn poke_before_next_suspend(&self, f: impl FnOnce(&RequestHandle) + Send + 'static) {
        *self.shared.pre_suspend_poke.lock().unwrap() = Some(Box::new(f));
    }
}

/// One scheduled timer callback: the only host I/O source in this milestone.
/// `interval` is Some for setInterval timers; interval timers never extend
/// the request lifetime.
struct TimerEntry {
    deadline: Instant,
    callback: Persistent<Function<'static>>,
    args: Vec<Persistent<Value<'static>>>,
    interval: Option<u64>,
}

/// A due timer handed to the JS segment. One-shots are removed from the map
/// before their callback runs; intervals stay registered (the callback may
/// clearInterval itself) and are rescheduled after it.
enum DueTimer {
    OneShot(TimerEntry),
    Interval {
        id: u64,
        callback: Persistent<Function<'static>>,
        args: Vec<Persistent<Value<'static>>>,
        interval_ms: u64,
    },
}

/// Task-local host state, shared with the binding closures on this thread.
#[derive(Default)]
struct HostState {
    timers: RefCell<HashMap<u64, TimerEntry>>,
    next_timer_id: Cell<u64>,
    wait_until_pending: Cell<usize>,
}

impl HostState {
    fn register_timer(&self, args: Rest<Value<'_>>, repeat: bool) -> rquickjs::Result<u64> {
        let mut args = args.0.into_iter();
        let callback = args.next().ok_or_else(|| {
            rquickjs::Error::new_from_js_message(
                "undefined",
                "function",
                "timer callback is required",
            )
        })?;
        let ctx = callback.ctx().clone();
        let callback = Function::from_js(&ctx, callback)?;
        let delay = timer_delay(&ctx, args.next())?;
        let id = self.next_timer_id.get() + 1;
        self.next_timer_id.set(id);
        self.timers.borrow_mut().insert(
            id,
            TimerEntry {
                deadline: Instant::now() + Duration::from_millis(delay),
                callback: Persistent::save(&ctx, callback),
                args: args.map(|v| Persistent::save(&ctx, v)).collect(),
                interval: repeat.then_some(delay),
            },
        );
        Ok(id)
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.timers.borrow().values().map(|t| t.deadline).min()
    }

    /// The next one-shot timer deadline. Interval timers are excluded: they
    /// fire while the request is alive for other reasons but never keep it
    /// alive on their own (spec 01 line 32).
    fn next_oneshot_deadline(&self) -> Option<Instant> {
        self.timers
            .borrow()
            .values()
            .filter(|t| t.interval.is_none())
            .map(|t| t.deadline)
            .min()
    }

    fn has_due_timer(&self) -> bool {
        let now = Instant::now();
        self.timers.borrow().values().any(|t| t.deadline <= now)
    }

    fn take_due_timer(&self) -> Option<DueTimer> {
        let now = Instant::now();
        let id = {
            let timers = self.timers.borrow();
            timers
                .iter()
                .filter(|(_, t)| t.deadline <= now)
                .min_by_key(|(_, t)| t.deadline)
                .map(|(id, _)| *id)
        }?;
        let mut timers = self.timers.borrow_mut();
        match timers.get(&id).map(|t| t.interval) {
            Some(Some(interval_ms)) => {
                let callback = timers.get(&id).expect("taken id").callback.clone();
                let args = timers.get(&id).expect("taken id").args.clone();
                Some(DueTimer::Interval {
                    id,
                    callback,
                    args,
                    interval_ms,
                })
            }
            Some(None) => Some(DueTimer::OneShot(timers.remove(&id).expect("taken id"))),
            None => None,
        }
    }
}

/// waitUntil stays open while the request is active; the background window
/// is fixed and never extended by new registrations.
fn wait_until_open(phase: Phase) -> bool {
    matches!(
        phase,
        Phase::Running | Phase::Responding | Phase::Background
    )
}

/// Convert the optional HTML timer delay to a non-negative integer number of
/// milliseconds. Missing, non-finite, and negative values are clamped to zero.
fn timer_delay<'js>(ctx: &Ctx<'js>, value: Option<Value<'js>>) -> rquickjs::Result<u64> {
    let Some(value) = value else {
        return Ok(0);
    };
    let number = Coerced::<f64>::from_js(ctx, value)?.0;
    if !number.is_finite() || number <= 0.0 {
        return Ok(0);
    }
    Ok(number.min(u64::MAX as f64).trunc() as u64)
}

enum EntryOutcome {
    Sync(HostResponse),
    Async(Persistent<Promise<'static>>),
}

/// Extract a settled fetch() result into the owned host response: a Response
/// instance yields status/headers/body; a plain string keeps the legacy
/// 200 status and empty headers (the M2 stub semantics, adapted not broken).
fn host_response<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<HostResponse> {
    if let Ok(response) = Class::<crate::web::response::Response>::from_js(ctx, value.clone()) {
        let (status, headers, body) = response.borrow().extract();
        return Ok(HostResponse {
            web_socket: None,
            status,
            headers,
            body,
            stream: None,
        });
    }
    let text = stringify(ctx, value)?;
    Ok(HostResponse {
        web_socket: None,
        status: 200,
        headers: Vec::new(),
        body: text.into_bytes(),
        stream: None,
    })
}

/// What may wake a suspended request task, besides its queued timer wakeup.
#[derive(Clone, Copy)]
enum WakeOn {
    /// Running/Background: cancellation only. Background must not poll
    /// `response_finished` (already set there): that would spin the loop
    /// into a busy poll.
    CancelOnly,
    /// Responding only: cancellation or the response being fully sent.
    CancelOrResponseFinished,
}

/// Parks the request task until poked (or until its timer wakeup fires).
struct Suspend {
    shared: Arc<Shared>,
    mode: WakeOn,
    exec: ExecutorHandle,
    wake_at: Option<Instant>,
}

impl Suspend {
    fn new(
        shared: &Arc<Shared>,
        mode: WakeOn,
        exec: ExecutorHandle,
        wake_at: Option<Instant>,
    ) -> Self {
        Suspend {
            shared: Arc::clone(shared),
            mode,
            exec,
            wake_at,
        }
    }

    fn ready(&self) -> bool {
        if self.shared.cancel.load(Ordering::SeqCst) || self.shared.io_ready.load(Ordering::SeqCst)
        {
            return true;
        }
        // A timer wakeup that already fired makes the task runnable, so the
        // main loop resumes and executes the due callbacks in a JS segment.
        if self.wake_at.is_some_and(|at| at <= Instant::now()) {
            return true;
        }
        match self.mode {
            WakeOn::CancelOnly => false,
            WakeOn::CancelOrResponseFinished => {
                self.shared.response_finished_at.lock().unwrap().is_some()
            }
        }
    }
}

impl Future for Suspend {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<()> {
        if self.ready() {
            return Poll::Ready(());
        }
        // Test hook (D1 regression): inject a poke after the first readiness
        // check but before the waker is registered.
        let hook = self.shared.pre_suspend_poke.lock().unwrap().take();
        if let Some(hook) = hook {
            hook(&RequestHandle {
                shared: Arc::clone(&self.shared),
            });
        }
        // Dekker side B: register the waker first...
        *self.shared.task_waker.lock().unwrap() = Some(cx.waker().clone());
        if let Some(at) = self.wake_at {
            self.exec.schedule_wakeup(at, cx.waker().clone());
        }
        // ...then re-check readiness: a poke that ran between the first check
        // and the registration must be observed here, or its wakeup is lost
        // and the task would park forever.
        if self.ready() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

/// One-shot fairness yield: re-queue the task so sibling request tasks run.
/// The slice budget is kept, not reset, across the yield.
struct Yield {
    yielded: bool,
}

impl Future for Yield {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<()> {
        if self.yielded {
            return Poll::Ready(());
        }
        self.yielded = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

/// Owns the Engine (and its admission permit) for the whole request life.
/// `!Send`: it lives and dies on its worker thread.
pub struct RequestTask {
    engine: Engine,
    shared: Arc<Shared>,
    exec: ExecutorHandle,
    host: Rc<HostState>,
    code: crate::bytecode::CompiledArtifact,
    io: Option<Rc<super::io::Io>>,
    call: Invocation,
}

impl RequestTask {
    pub(crate) fn new(
        engine: Engine,
        exec: ExecutorHandle,
        code: &crate::bytecode::CompiledArtifact,
        call: Invocation,
    ) -> (RequestHandle, RequestTask) {
        let shared = Shared::new(
            engine.cancellation(),
            engine.clock().deadline_handle(),
            engine.limits().background_ms,
        );
        let handle = RequestHandle {
            shared: Arc::clone(&shared),
        };
        let task = RequestTask {
            engine,
            shared,
            exec,
            host: Rc::new(HostState::default()),
            code: code.clone(),
            io: None,
            call,
        };
        (handle, task)
    }

    pub fn with_io(mut self, config: super::io::IoConfig) -> Self {
        self.io = Some(super::io::Io::new(
            config,
            RequestHandle {
                shared: self.shared.clone(),
            },
        ));
        *self.shared.host_budget.lock().unwrap() = self.io.as_ref().map(|io| io.budget());
        self
    }
    /// Drive the lifecycle to Closed. Runs on the owning worker thread,
    /// spawned into that thread's `Executor`.
    pub async fn drive(mut self) {
        let outcome = self.lifecycle().await.map_err(|e| format!("{e:#}"));
        self.teardown();
        *self.shared.outcome.lock().unwrap() = Some(outcome);
        // The Engine drops here: Context -> Runtime -> permit, exactly once.
    }

    async fn lifecycle(&mut self) -> Result<String> {
        self.check_cancel()?;
        self.set_phase(Phase::Running);
        // First JS segment: bindings, module load/evaluation, fetch() entry.
        self.engine.clock().start_segment();
        let response = match self.call_entry()? {
            EntryOutcome::Sync(response) => response,
            EntryOutcome::Async(promise) => self.settle_entry(promise).await?,
        };
        // D2: write the response slot BEFORE flipping to Responding; a reader
        // that observes Responding under the lock must see the response.
        self.shared.js_memory.store(
            self.engine.runtime().memory_usage().memory_used_size.max(0) as usize,
            Ordering::Relaxed,
        );
        let body = String::from_utf8_lossy(&response.body).into_owned();
        *self.shared.response.lock().unwrap() = Some(response);
        self.set_phase(Phase::Responding);
        let _ = self.shared.response_ready.try_send(());
        let response_was_finished = self.shared.response_finished_at.lock().unwrap().is_some();
        if !response_was_finished {
            self.engine.clock().host_wait();
        }
        // Keep JS transforms and host completions progressing while HTTP reads.
        while self.shared.response_finished_at.lock().unwrap().is_none() {
            if self.io.is_some() {
                self.run_ready().await?;
            }
            self.engine.clock().host_wait();
            Suspend::new(
                &self.shared,
                WakeOn::CancelOrResponseFinished,
                self.exec.clone(),
                if self.io.is_some() {
                    self.host.next_deadline()
                } else {
                    None
                },
            )
            .await;
            self.check_cancel()?;
        }
        self.check_cancel()?;
        let response_finished_at = self
            .shared
            .response_finished_at
            .lock()
            .unwrap()
            .unwrap_or_else(Instant::now);
        let deadline =
            response_finished_at + Duration::from_millis(self.engine.limits().background_ms);
        self.engine.clock().set_deadline(deadline);
        match self.finish_work(deadline).await {
            Ok(()) => {}
            Err(error) if error.is::<BackgroundExpired>() => {}
            Err(error) => return Err(error),
        }
        Ok(body)
    }

    async fn finish_work(&mut self, deadline: Instant) -> Result<()> {
        // Ready jobs after a synchronous response obey the same deadline as
        // later waitUntil work, including interruption inside a running job.
        self.engine.clock().start_segment();
        self.drain_jobs().await?;
        self.engine.clock().host_wait();
        if self.host.wait_until_pending.get() > 0 {
            self.background(deadline).await?;
        }
        Ok(())
    }

    /// Background: drive waitUntil work inside the fixed window. The window
    /// starts when the response finishes and is never extended by new
    /// registrations; cancellation and forced teardown do not extend it.
    async fn background(&mut self, deadline: Instant) -> Result<()> {
        self.set_phase(Phase::Background);
        loop {
            self.run_ready().await?;
            self.engine.clock().host_wait();
            if self.host.wait_until_pending.get() == 0 {
                return Ok(());
            }
            let wake = self
                .host
                .next_deadline()
                .map_or(deadline, |t| t.min(deadline));
            Suspend::new(
                &self.shared,
                WakeOn::CancelOnly,
                self.exec.clone(),
                Some(wake),
            )
            .await;
        }
    }

    fn set_phase(&self, phase: Phase) {
        *self.shared.phase.lock().unwrap() = phase;
    }

    fn phase(&self) -> Phase {
        *self.shared.phase.lock().unwrap()
    }

    fn check_cancel(&self) -> Result<()> {
        if self.shared.cancel.load(Ordering::SeqCst) {
            return Err(anyhow!("request cancelled"));
        }
        Ok(())
    }

    fn call_entry(&mut self) -> Result<EntryOutcome> {
        if self.call.method == "__DO" {
            let guard = Rc::new(Cell::new(false));
            self.engine
                .runtime()
                .set_promise_hook(Some(Box::new(move |ctx, kind, p, parent| {
                    if guard.replace(true) {
                        return;
                    }
                    if let Ok(hook) = ctx.globals().get::<_, Function>("__gatewayPromiseHook")
                        && hook.call::<_, ()>((kind as u8, p, parent)).is_err()
                    {
                        let _ = ctx.catch();
                    }
                    guard.set(false);
                })));
        }
        self.engine.context().with(|ctx| -> Result<EntryOutcome> {
            if let Some(io) = &self.io {
                io.install(&ctx)?;
            }
            let globals = ctx.globals();
            // Minimal host bindings: timers are the only host I/O source here.
            for (name, repeat) in [("setTimeout", false), ("setInterval", true)] {
                let host = Rc::clone(&self.host);
                globals.set(
                    name,
                    Function::new(ctx.clone(), move |args: Rest<Value>| {
                        host.register_timer(args, repeat)
                    })?,
                )?;
            }
            let host = Rc::clone(&self.host);
            let clear_timeout = Function::new(ctx.clone(), move |id: u64| {
                host.timers.borrow_mut().remove(&id);
            })?;
            globals.set("clearTimeout", clear_timeout.clone())?;
            // clearInterval is the same removal as clearTimeout; the IDs
            // share one namespace, as in the standard.
            globals.set("clearInterval", clear_timeout)?;
            // queueMicrotask is built into quickjs-ng 0.16.2.

            let exec = Object::new(ctx.clone())?;
            // Registering `then` from Rust would lose the receiver; a tiny JS
            // helper keeps `promise.then(cb, cb)` bound to its promise.
            ctx.eval::<(), _>(
                "globalThis.__gatewayWaitUntil = function (p, cb) { p.then(cb, cb); };",
            )?;
            let host = Rc::clone(&self.host);
            let shared = Arc::clone(&self.shared);
            let wait_until =
                Function::new(ctx.clone(), move |promise: Value| -> rquickjs::Result<()> {
                    // Registration is only open while the request is active.
                    if !wait_until_open(*shared.phase.lock().unwrap()) {
                        return Err(rquickjs::Exception::throw_type(
                            promise.ctx(),
                            "waitUntil() called outside the active request",
                        ));
                    }
                    let Some(promise) = promise.as_promise() else {
                        // A non-promise value settles immediately (Workers
                        // wraps it with Promise.resolve).
                        return Ok(());
                    };
                    host.wait_until_pending
                        .set(host.wait_until_pending.get() + 1);
                    let on_settled = Function::new(promise.ctx().clone(), {
                        let host = Rc::clone(&host);
                        move || {
                            host.wait_until_pending
                                .set(host.wait_until_pending.get() - 1);
                        }
                    })?;
                    // Decrement on either settle path; the derived promise is
                    // dropped and reclaimed with the request runtime.
                    let register: Function = promise.ctx().globals().get("__gatewayWaitUntil")?;
                    register.call::<_, ()>((promise.clone(), on_settled))?;
                    Ok(())
                })?;
            exec.set("waitUntil", wait_until)?;
            let returned = call_entry(&ctx, &self.code, &self.call, exec)?;
            if let Some(promise) = returned.as_promise() {
                return Ok(EntryOutcome::Async(Persistent::save(&ctx, promise.clone())));
            }
            Ok(EntryOutcome::Sync(self.extract_response(&ctx, returned)?))
        })
    }

    /// Running: settle the entry promise while host work is outstanding.
    async fn settle_entry(
        &mut self,
        promise: Persistent<Promise<'static>>,
    ) -> Result<HostResponse> {
        loop {
            self.run_ready().await?;
            // About to suspend on host work: stop the clock; the next segment
            // gets a fresh budget.
            self.engine.clock().host_wait();
            match self.promise_state(&promise) {
                PromiseState::Resolved => return self.promise_body(&promise),
                PromiseState::Rejected => return Err(self.promise_rejection(&promise)),
                PromiseState::Pending => {}
            }
            // Only one-shot timers count as host work that keeps the entry
            // promise worth waiting for; intervals never extend the request.
            match self.host.next_oneshot_deadline() {
                Some(_) => {
                    // Wake at the earliest timer of any kind so intervals fire
                    // while the request is alive for the one-shot.
                    let at = self.host.next_deadline().expect("one-shot exists");
                    Suspend::new(
                        &self.shared,
                        WakeOn::CancelOnly,
                        self.exec.clone(),
                        Some(at),
                    )
                    .await;
                }
                None if self.io.as_ref().is_some_and(|io| io.pending()) => {
                    Suspend::new(
                        &self.shared,
                        WakeOn::CancelOnly,
                        self.exec.clone(),
                        self.host.next_deadline(),
                    )
                    .await;
                }
                None => {
                    // A dangling promise with no host task behind it can never
                    // be woken: report unfinished execution, do not wait forever.
                    return Err(anyhow!(
                        "unfinished execution: pending promise with no host task"
                    ));
                }
            }
        }
    }

    fn promise_state(&self, promise: &Persistent<Promise<'static>>) -> PromiseState {
        self.engine.context().with(|ctx| {
            promise
                .clone()
                .restore(&ctx)
                .expect("promise belongs to this runtime")
                .state()
        })
    }

    fn promise_body(&self, promise: &Persistent<Promise<'static>>) -> Result<HostResponse> {
        self.engine.context().with(|ctx| -> Result<HostResponse> {
            let promise = promise
                .clone()
                .restore(&ctx)
                .map_err(|e| anyhow!("restore promise: {e}"))?;
            let value: Value = promise
                .result()
                .expect("state was Resolved")
                .map_err(|e| describe(&ctx, "fetch()", e))?;
            self.extract_response(&ctx, value)
        })
    }

    fn promise_rejection(&self, promise: &Persistent<Promise<'static>>) -> anyhow::Error {
        self.engine.context().with(|ctx| {
            let restored = promise.clone().restore(&ctx);
            match restored {
                Ok(promise) => match promise.result::<Value>() {
                    Some(Err(err)) => describe(&ctx, "fetch()", err),
                    other => anyhow!("fetch() rejected: {other:?}"),
                },
                Err(err) => anyhow!("restore promise: {err}"),
            }
        })
    }

    fn extract_response<'js>(&self, ctx: &Ctx<'js>, value: Value<'js>) -> Result<HostResponse> {
        if let Some(io) = &self.io {
            let obj = Object::from_js(ctx, value)?;
            let status = obj.get("status")?;
            let raw: rquickjs::Array = obj.get("headers")?;
            let headers = raw
                .iter::<rquickjs::Array>()
                .map(|row| {
                    let row = row?;
                    Ok((row.get(0)?, row.get(1)?))
                })
                .collect::<rquickjs::Result<Vec<_>>>()?;
            let id: Option<u64> = obj.get("body")?;
            let socket: Option<u64> = obj.get("webSocket")?;
            Ok(HostResponse {
                web_socket: socket.map(|id| io.socket(id)).transpose()?,
                status,
                headers,
                body: Vec::new(),
                stream: id.map(|id| io.body(id)).transpose()?,
            })
        } else {
            host_response(ctx, value)
        }
    }
    /// Every job/callback passes this gate, including after a fairness yield.
    fn check_execution(&self) -> Result<()> {
        self.check_cancel()?;
        if self.engine.clock().deadline_expired() {
            return Err(BackgroundExpired.into());
        }
        if self.engine.interrupted() {
            return Err(anyhow!("JavaScript execution slice exceeded"));
        }
        Ok(())
    }

    async fn run_ready(&mut self) -> Result<()> {
        self.check_execution()?;
        self.engine.clock().start_segment();
        if let Some(io) = &self.io {
            self.engine.context().with(|ctx| io.complete(&ctx))?;
        }
        self.fire_due_timers()?;
        let result = self.drain_jobs().await;
        self.shared.js_memory.store(
            self.engine.runtime().memory_usage().memory_used_size.max(0) as usize,
            Ordering::Relaxed,
        );
        result
    }

    fn fire_due_timers(&mut self) -> Result<()> {
        if let Some(due) = self.host.take_due_timer() {
            // Late callbacks check the phase and never touch a closed request.
            if self.phase() != Phase::Closed {
                self.check_execution()?;
                let (callback, callback_args, reschedule) = match &due {
                    DueTimer::OneShot(entry) => (entry.callback.clone(), entry.args.clone(), None),
                    DueTimer::Interval {
                        id,
                        callback,
                        args,
                        interval_ms,
                    } => (callback.clone(), args.clone(), Some((*id, *interval_ms))),
                };
                let result = self.engine.context().with(|ctx| -> Result<()> {
                    let callback = callback
                        .restore(&ctx)
                        .map_err(|e| anyhow!("restore timer callback: {e}"))?;
                    let mut args = Args::new(ctx.clone(), callback_args.len());
                    for value in callback_args {
                        args.push_arg(
                            value
                                .restore(&ctx)
                                .map_err(|e| anyhow!("restore timer argument: {e}"))?,
                        )?;
                    }
                    args.apply::<()>(&callback)
                        .map_err(|e| describe(&ctx, "timer callback", e))
                });
                self.check_execution()?;
                result?;
                if let Some((id, interval_ms)) = reschedule {
                    // A clearInterval inside the callback removed the entry; only
                    // reschedule a timer that is still registered.
                    if let Some(entry) = self.host.timers.borrow_mut().get_mut(&id) {
                        entry.deadline = Instant::now() + Duration::from_millis(interval_ms);
                    }
                }
            }
        }
        Ok(())
    }

    async fn drain_jobs(&mut self) -> Result<()> {
        let mut jobs = 0u64;
        self.check_execution()?;
        while self.engine.runtime().is_job_pending() {
            self.check_execution()?;
            let result = self.engine.runtime().execute_pending_job();
            self.check_execution()?;
            result.map_err(|e| anyhow!("pending job failed: {e:?}"))?;
            jobs += 1;
            if jobs.is_multiple_of(64) && !self.host.has_due_timer() {
                // Fairness yield: keep the accumulated slice budget and let
                // sibling request tasks run.
                self.engine.clock().yield_pause();
                Yield { yielded: false }.await;
                self.engine.clock().start_segment();
            }
        }
        Ok(())
    }

    /// Closing: cancel timers, drop host-held JS references, flip to Closed.
    /// Failures, cancellation and timeouts never drain plugin jobs to finish.
    fn teardown(&mut self) {
        self.set_phase(Phase::Closing);
        if self.shared.cancel.load(Ordering::SeqCst)
            && let Some(io) = &self.io
        {
            self.engine.context().with(|ctx| io.abort_signal(&ctx));
        }
        self.host.timers.borrow_mut().clear();
        if let Some(io) = &self.io {
            io.clear();
        }
        self.shared.response_ready.close();
        self.set_phase(Phase::Closed);
        self.shared.closed.close();
    }
}
