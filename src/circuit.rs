//! In-memory circuit breaker for the local model. The clock is passed in, so it is pure to test.

use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
enum State {
    Closed {
        fails: u32,
    },
    Open {
        until: Instant,
    },
    /// One probe is in flight; if it never reports back, the lease expires and another may probe.
    HalfOpen {
        lease_until: Instant,
    },
}

pub struct Breaker {
    threshold: u32,
    cooldown: Duration,
    state: Mutex<State>,
}

impl Breaker {
    /// Opens after `threshold` consecutive failures; 0 disables it.
    pub fn new(threshold: u32, cooldown: Duration) -> Self {
        Self {
            threshold,
            cooldown,
            state: Mutex::new(State::Closed { fails: 0 }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        // A panic while holding this lock cannot leave the state half-written; keep using it.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// May this request call the local model? True for at most one probe per cooldown when open.
    pub fn allow(&self, now: Instant) -> bool {
        let mut st = self.state();
        match *st {
            State::Closed { .. } => true,
            State::Open { until } | State::HalfOpen { lease_until: until } if now < until => false,
            State::Open { .. } | State::HalfOpen { .. } => {
                *st = State::HalfOpen {
                    lease_until: now + self.cooldown,
                };
                true
            }
        }
    }

    /// Outcome of a real call to the local model.
    pub fn record(&self, ok: bool, now: Instant) {
        let mut st = self.state();
        let open = State::Open {
            until: now + self.cooldown,
        };
        *st = match (*st, ok) {
            (_, true) => State::Closed { fails: 0 },
            (State::Closed { fails }, false)
                if self.threshold > 0 && fails + 1 >= self.threshold =>
            {
                open
            }
            (State::Closed { fails }, false) => State::Closed {
                fails: fails.saturating_add(1),
            },
            (State::HalfOpen { .. }, false) => open,
            // A late failure from a request sent before the breaker opened keeps the original until.
            (s @ State::Open { .. }, false) => s,
        };
    }
}
