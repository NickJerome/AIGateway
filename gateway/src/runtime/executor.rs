//! Per-worker-thread local executor: one `LocalPool`, request tasks as local
//! tasks, deadline-driven parking instead of busy-polling.

use futures::executor::{LocalPool, LocalSpawner};
use futures::task::LocalSpawnExt;
use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::future::Future;
use std::rc::Rc;
use std::task::Waker;
use std::time::Instant;

/// One queued wakeup: wake `waker` once `at` has passed. Stale entries
/// (already fired or superseded re-registrations) are dropped lazily; waking
/// a completed task's waker is harmless.
struct Wakeup {
    at: Instant,
    seq: u64,
    waker: Waker,
}

impl PartialEq for Wakeup {
    fn eq(&self, other: &Self) -> bool {
        self.at == other.at && self.seq == other.seq
    }
}
impl Eq for Wakeup {}
impl PartialOrd for Wakeup {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Wakeup {
    // Inverted: the heap top is the earliest deadline.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .at
            .cmp(&self.at)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

/// Scheduling side of the executor, shared with suspended request tasks.
#[derive(Clone)]
pub struct ExecutorHandle {
    spawner: LocalSpawner,
    live: Rc<Cell<usize>>,
    wakeups: Rc<RefCell<BinaryHeap<Wakeup>>>,
    seq: Rc<Cell<u64>>,
}

impl ExecutorHandle {
    pub fn spawn(&self, task: impl Future<Output = ()> + 'static) {
        self.live.set(self.live.get() + 1);
        let live = self.live.clone();
        self.spawner
            .spawn_local(async move {
                task.await;
                live.set(live.get() - 1);
            })
            .expect("executor alive");
    }
    /// Wake `waker` once `at` is reached. Only meaningful while the matching
    /// `Executor::run` is driving the pool.
    pub fn schedule_wakeup(&self, at: Instant, waker: Waker) {
        if self
            .wakeups
            .borrow()
            .iter()
            .any(|queued| queued.at <= at && queued.waker.will_wake(&waker))
        {
            return;
        }
        let seq = self.seq.get() + 1;
        self.seq.set(seq);
        self.wakeups.borrow_mut().push(Wakeup { at, seq, waker });
    }

    /// Drop every queued wakeup, used when the pool is torn down.
    pub fn clear_wakeups(&self) {
        self.wakeups.borrow_mut().clear();
    }
}

/// One local executor per worker thread. Request tasks hold their Engine and
/// permit for their whole life; the executor never moves a task off-thread.
pub struct Executor {
    pool: LocalPool,
    spawner: LocalSpawner,
    live: Rc<Cell<usize>>,
    wakeups: Rc<RefCell<BinaryHeap<Wakeup>>>,
    seq: Rc<Cell<u64>>,
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}

impl Executor {
    pub fn new() -> Self {
        let pool = LocalPool::new();
        let spawner = pool.spawner();
        Executor {
            pool,
            spawner,
            live: Rc::new(Cell::new(0)),
            wakeups: Rc::new(RefCell::new(BinaryHeap::new())),
            seq: Rc::new(Cell::new(0)),
        }
    }

    pub fn handle(&self) -> ExecutorHandle {
        ExecutorHandle {
            spawner: self.spawner.clone(),
            live: self.live.clone(),
            wakeups: Rc::clone(&self.wakeups),
            seq: Rc::clone(&self.seq),
        }
    }

    pub fn live_tasks(&self) -> usize {
        self.live.get()
    }

    /// Spawn a request task as a local task of this thread's pool.
    pub fn spawn(&self, task: impl Future<Output = ()> + 'static) {
        self.handle().spawn(task);
    }

    /// Drive every spawned task to completion. When no task is ready the
    /// thread parks: any task waker unparks it (the pool's waker targets this
    /// thread), and queued wakeups bound the park to the nearest deadline.
    /// There is no busy-poll loop over runtimes.
    pub fn run(&mut self) {
        loop {
            self.pool.run_until_stalled();
            self.fire_due_wakeups();
            // Fired wakeups made tasks ready: poll them before deciding to park.
            self.pool.run_until_stalled();
            if self.live.get() == 0 {
                return;
            }
            let next = self.wakeups.borrow().peek().map(|w| w.at);
            match next {
                Some(deadline) => {
                    let now = Instant::now();
                    if deadline > now {
                        std::thread::park_timeout(deadline - now);
                    }
                }
                None => std::thread::park(),
            }
        }
    }

    fn fire_due_wakeups(&mut self) {
        loop {
            let due = self
                .wakeups
                .borrow()
                .peek()
                .is_some_and(|w| w.at <= Instant::now());
            if !due {
                return;
            }
            let wakeup = self.wakeups.borrow_mut().pop().expect("peeked entry");
            wakeup.waker.wake();
        }
    }
}
