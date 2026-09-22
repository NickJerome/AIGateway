use crate::engine::Engine;
use crate::limits::Limits;
use anyhow::{Result, anyhow};
use std::cell::Cell;
use std::rc::Rc;

/// Owns admission for one worker thread; contains no QuickJS Runtime.
pub struct Worker {
    limits: Limits,
    in_flight: Rc<Cell<usize>>,
}

pub(crate) struct RequestPermit(Rc<Cell<usize>>);

impl Drop for RequestPermit {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

impl Default for Worker {
    fn default() -> Self {
        Self::new()
    }
}

impl Worker {
    pub fn new() -> Self {
        Self::with_limits(Limits::default())
    }

    pub fn with_limits(limits: Limits) -> Self {
        Self {
            limits,
            in_flight: Rc::new(Cell::new(0)),
        }
    }

    pub fn in_flight(&self) -> usize {
        self.in_flight.get()
    }

    pub fn start_request(&self) -> Result<Engine> {
        if self.in_flight() >= self.limits.max_in_flight {
            return Err(anyhow!(
                "in-flight limit reached: {}",
                self.limits.max_in_flight
            ));
        }
        self.in_flight.set(self.in_flight() + 1);
        let permit = RequestPermit(Rc::clone(&self.in_flight));
        Engine::new(self.limits, permit)
    }
}
