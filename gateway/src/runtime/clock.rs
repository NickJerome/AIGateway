//! Per-request accounting for continuous JavaScript execution slices.

use std::cell::Cell;
use std::time::{Duration, Instant};

/// Wall-clock budget for one request's JavaScript execution.
///
/// A segment is one uninterrupted stretch of JS work: module evaluation,
/// the entry call, a timer callback, or a drained batch of jobs. A host wait
/// stops the clock and hands the next segment a fresh budget; a fairness
/// yield only pauses the segment and keeps the accumulated budget, so a
/// microtask storm cannot yield its way around the limit.
///
/// Slice accounting is thread-local. The one-shot deadline can also be set
/// by HTTP completion while JavaScript is executing.
pub struct SliceClock {
    budget: Duration,
    deadline: std::sync::Arc<std::sync::OnceLock<Instant>>,
    accumulated: Cell<Duration>,
    segment: Cell<Option<Instant>>,
}

impl SliceClock {
    pub fn new(budget_ms: u64) -> Self {
        SliceClock {
            budget: Duration::from_millis(budget_ms),
            deadline: Default::default(),
            accumulated: Cell::new(Duration::ZERO),
            segment: Cell::new(None),
        }
    }

    /// Absolute response-completion deadline; host waits never reset it.
    pub fn set_deadline(&self, deadline: Instant) {
        let _ = self.deadline.set(deadline);
    }

    pub(crate) fn deadline_handle(&self) -> std::sync::Arc<std::sync::OnceLock<Instant>> {
        self.deadline.clone()
    }

    pub fn deadline_expired(&self) -> bool {
        self.deadline.get().is_some_and(|at| Instant::now() >= *at)
    }

    /// Begin or resume a JS segment.
    pub fn start_segment(&self) {
        if self.segment.get().is_none() {
            self.segment.set(Some(Instant::now()));
        }
    }

    /// Pause for a host wait: the next segment starts with a fresh budget.
    pub fn host_wait(&self) {
        self.accumulated.set(Duration::ZERO);
        self.segment.set(None);
    }

    /// Pause for a fairness yield: the accumulated budget is kept.
    pub fn yield_pause(&self) {
        if let Some(start) = self.segment.take() {
            self.accumulated
                .set(self.accumulated.get() + start.elapsed());
        }
    }

    /// Total JS time charged to the current slice.
    pub fn elapsed(&self) -> Duration {
        let running = self.segment.get().map_or(Duration::ZERO, |s| s.elapsed());
        self.accumulated.get() + running
    }

    /// True once the slice budget is exhausted; read by the interrupt handler.
    pub fn expired(&self) -> bool {
        self.elapsed() >= self.budget
    }
}
