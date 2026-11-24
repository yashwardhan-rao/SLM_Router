//! One log record per request, even when the client hangs up before we answer.

use super::AppState;
use crate::logging::Record;
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
    time::Instant,
};

/// Owns the request's record. If the handler future is dropped (the client closed the
/// connection while we waited on an upstream), `Drop` logs it as a `client_disconnect`.
pub struct Guard {
    s: Arc<AppState>,
    started: Instant,
    rec: Option<Record>,
}

impl Guard {
    pub fn new(s: Arc<AppState>, rec: Record, started: Instant) -> Self {
        Self {
            s,
            started,
            rec: Some(rec),
        }
    }

    /// Write the finished record now.
    pub fn log(mut self) {
        if let Some(mut rec) = self.rec.take() {
            rec.total_ms = self.started.elapsed().as_millis() as u64;
            self.s.logger.write(&rec);
        }
    }

    /// Hand the record to the stream relay, which logs it when the stream ends.
    pub fn into_record(mut self) -> Record {
        self.rec
            .take()
            .expect("the record is only taken by a method that consumes the guard")
    }
}

impl Deref for Guard {
    type Target = Record;
    fn deref(&self) -> &Record {
        self.rec
            .as_ref()
            .expect("the record is only taken by a method that consumes the guard")
    }
}

impl DerefMut for Guard {
    fn deref_mut(&mut self) -> &mut Record {
        self.rec
            .as_mut()
            .expect("the record is only taken by a method that consumes the guard")
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(mut rec) = self.rec.take() {
            // 499: the client closed the request (nginx convention); nothing was sent.
            (rec.status, rec.outcome) = (499, "client_disconnect");
            rec.total_ms = self.started.elapsed().as_millis() as u64;
            self.s.logger.write(&rec);
        }
    }
}
