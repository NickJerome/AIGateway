//! Fixed thread-local JS workers, admission capacity and shutdown.
use crate::{
    config::Config,
    engine::Invocation,
    network::Network,
    plugins::Plugin,
    runtime::{
        executor::Executor,
        io::{DurableIo, IoConfig},
        request::{Phase, RequestHandle},
    },
    worker::Worker,
};
use anyhow::Result;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::body::ResponseGuard;
struct Job {
    plugin: Arc<Plugin>,
    call: Invocation,
    io: IoConfig,
    reply: tokio::sync::oneshot::Sender<Result<ResponseGuard>>,
    _capacity: OwnedSemaphorePermit,
    admission: Option<OwnedSemaphorePermit>,
}
struct Slot {
    admission: Arc<Semaphore>,
    queue: async_channel::Sender<Job>,
    capacity: Arc<Semaphore>,
}
pub(super) struct Workers {
    slots: Vec<Slot>,
    next: AtomicUsize,
    threads: Mutex<Vec<std::thread::JoinHandle<()>>>,
    active: Arc<Mutex<Vec<RequestHandle>>>,
    pub(super) durable: Arc<crate::durable::Router>,
    do_active: Arc<Mutex<Vec<RequestHandle>>>,
    pub(super) ws_capacity: Arc<Semaphore>,
}
impl Workers {
    pub(super) fn new(config: &Config, telemetry: crate::telemetry::Telemetry) -> Result<Self> {
        let ws_capacity = Arc::new(Semaphore::new(config.websocket.max_connections));
        let (durable, receivers) =
            crate::durable::Router::new(config.threads, config.http.host_buffer_bytes);
        let mut slots = Vec::new();
        let mut threads = Vec::new();
        let active = Arc::new(Mutex::new(Vec::<RequestHandle>::new()));
        let do_active = Arc::new(Mutex::new(Vec::<RequestHandle>::new()));
        for (id, do_rx) in receivers.into_iter().enumerate() {
            let router = durable.clone();
            let telemetry = telemetry.clone();
            let websocket = config.websocket.clone();
            let socket_capacity = ws_capacity.clone();
            let runtime = tokio::runtime::Handle::current();
            let network = Network::new(config.http);
            let admission = Arc::new(Semaphore::new(config.limits.max_in_flight));
            let thread_admission = admission.clone();
            let limits = config.limits;
            let (queue, rx) = async_channel::bounded::<Job>(limits.max_in_flight);
            let handles = active.clone();
            let do_handles = do_active.clone();
            threads.push(
                std::thread::Builder::new()
                    .name(format!("worker-{id}"))
                    .spawn(move || {
                        let mut executor = Executor::new();
                        let exec = executor.handle();
                        let do_exec = exec.clone();

                        executor.spawn(async move {
                            while let Ok(start) = do_rx.recv().await {
                                let Ok(engine) = crate::engine::Engine::durable(limits) else {
                                    start.service.close();
                                    continue;
                                };
                                let (handle, task) = engine.into_task(
                                    &do_exec,
                                    &start.plugin.code,
                                    Invocation {
                                        url: "http://durable/".into(),
                                        method: "__DO".into(),
                                    },
                                );
                                if start.plugin.attach(handle.clone()).is_err() {
                                    start.service.close();
                                    continue;
                                }
                                {
                                    let mut handles = do_handles.lock().unwrap();
                                    handles.retain(|h| h.phase() != Phase::Closed);
                                    handles.push(handle);
                                }
                                let io = IoConfig {
                                    identity: None,
                                    websocket: websocket.clone(),
                                    ws_capacity: socket_capacity.clone(),
                                    upgrade: false,
                                    telemetry: Some(telemetry.clone()),
                                    request_id: "durable".into(),
                                    trace: None,
                                    network: network.clone(),
                                    runtime: runtime.clone(),
                                    vars: start.plugin.vars.clone(),
                                    bindings: start.plugin.bindings.clone(),
                                    headers: vec![],
                                    body: None,
                                    durable: Some(DurableIo {
                                        router: router.clone(),
                                        plugin: start.plugin,
                                        service: Some(start.service),
                                    }),
                                };
                                do_exec.spawn(task.with_io(io).drive());
                            }
                        });
                        let worker = std::rc::Rc::new(Worker::with_limits(limits));
                        let admission = thread_admission;
                        executor.spawn(async move {
                            while let Ok(mut job) = rx.recv().await {
                                let worker = worker.clone();
                                let admission = admission.clone();
                                let handles = handles.clone();
                                let request_exec = exec.clone();
                                // Each queued task still owns a bounded capacity permit.
                                // Cron has already reserved admission and cannot sit behind
                                // an HTTP task that is waiting for that same permit.
                                exec.spawn(async move {
                                    let permit = match job.admission.take() {
                                        Some(p) => p,
                                        None => admission
                                            .acquire_owned()
                                            .await
                                            .expect("worker admission"),
                                    };
                                    if job.reply.is_closed() {
                                        return;
                                    }
                                    let engine = match worker.start_request() {
                                        Ok(e) => e,
                                        Err(e) => {
                                            let _ = job.reply.send(Err(e));
                                            return;
                                        }
                                    };
                                    let (handle, task) =
                                        engine.into_task(&request_exec, &job.plugin.code, job.call);
                                    if let Err(e) = job.plugin.attach(handle.clone()) {
                                        let _ = job.reply.send(Err(e));
                                        return;
                                    }
                                    {
                                        let mut active = handles.lock().unwrap();
                                        active.retain(|h| h.phase() != Phase::Closed);
                                        active.push(handle.clone());
                                    }
                                    let task = task.with_io(job.io);
                                    let _ = job.reply.send(Ok(ResponseGuard {
                                        handle,
                                        finished: false,
                                        trace: None,
                                    }));
                                    let _permit = permit;
                                    let _capacity = job._capacity;
                                    task.drive().await;
                                });
                            }
                        });
                        executor.run();
                    })?,
            );
            slots.push(Slot {
                admission,
                queue,
                capacity: Arc::new(Semaphore::new(limits.max_in_flight.saturating_mul(2))),
            });
        }
        Ok(Self {
            slots,
            next: AtomicUsize::new(0),
            threads: Mutex::new(threads),
            active,
            durable,
            do_active,
            ws_capacity,
        })
    }
    /// Reserves capacity before reading an HTTP body. Dropping releases it automatically.
    pub(super) fn reserve(&self) -> Option<Reservation> {
        let start = self.next.fetch_add(1, Ordering::Relaxed);
        (0..self.slots.len()).find_map(|offset| {
            let slot = &self.slots[start.wrapping_add(offset) % self.slots.len()];
            Some(Reservation {
                queue: slot.queue.clone(),
                capacity: slot.capacity.clone().try_acquire_owned().ok()?,
                admission: None,
            })
        })
    }

    /// Cron must reserve execution admission immediately, so it cannot wait behind
    /// an HTTP job that needs the same permit. Stable affinity preserves DO locality.
    pub(super) fn reserve_scheduled(&self, affinity: &str) -> Option<Reservation> {
        let slot = &self.slots[crate::durable::owner(affinity, self.slots.len())];
        let admission = slot.admission.clone().try_acquire_owned().ok()?;
        let capacity = slot.capacity.clone().try_acquire_owned().ok()?;
        Some(Reservation {
            queue: slot.queue.clone(),
            capacity,
            admission: Some(admission),
        })
    }

    pub(super) fn memory(&self) -> ((usize, usize, usize), (usize, usize, usize)) {
        fn usage(handles: &Mutex<Vec<RequestHandle>>) -> (usize, usize, usize) {
            handles
                .lock()
                .unwrap()
                .iter()
                .filter(|h| h.phase() != Phase::Closed)
                .fold((0, 0, 0), |(n, js, host), h| {
                    let (a, b) = h.memory();
                    (n + 1, js + a, host + b)
                })
        }
        (usage(&self.active), usage(&self.do_active))
    }

    pub(super) async fn drained(&self) {
        let active = self.active.lock().unwrap().clone();
        for handle in active {
            handle.closed().await;
        }
    }
    pub(super) fn close(&self) {
        for slot in &self.slots {
            slot.queue.close();
        }
    }
    pub(super) fn cancel(&self) {
        self.durable.close();
        for h in self.do_active.lock().unwrap().iter() {
            h.cancel();
        }
        for h in self.active.lock().unwrap().iter() {
            h.cancel();
        }
    }
    pub(super) async fn join(&self) {
        let threads = std::mem::take(&mut *self.threads.lock().unwrap());
        let _ = tokio::task::spawn_blocking(move || {
            for t in threads {
                let _ = t.join();
            }
        })
        .await;
    }
}

/// Owns both the destination and its permits: callers cannot mix worker slots or
/// enqueue an uncharged task. Submission consumes the reservation exactly once.
pub(super) struct Reservation {
    queue: async_channel::Sender<Job>,
    capacity: OwnedSemaphorePermit,
    admission: Option<OwnedSemaphorePermit>,
}
impl Reservation {
    pub(super) fn submit(
        self,
        plugin: Arc<Plugin>,
        call: Invocation,
        io: IoConfig,
    ) -> Option<tokio::sync::oneshot::Receiver<Result<ResponseGuard>>> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        let job = Job {
            plugin,
            call,
            io,
            reply,
            _capacity: self.capacity,
            admission: self.admission,
        };
        self.queue.try_send(job).ok()?;
        Some(rx)
    }
}
