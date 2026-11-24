//! Append-only JSONL request log. Metadata only: never prompt text, completions or keys.

use crate::{
    routing::{Policy, Reason, Target},
    session::SessionSource,
};
use serde::Serialize;
use std::{io::Write, path::PathBuf};

#[derive(Debug, Serialize)]
pub struct Record {
    pub v: u8,
    /// Unix epoch milliseconds.
    pub ts: u64,
    pub request_id: String,
    pub session_id: String,
    pub session_source: SessionSource,
    pub policy: Policy,
    pub target: Target,
    pub reason: Reason,
    pub failed_over: bool,
    pub pinned: bool,
    pub stream: bool,
    pub status: u16,
    pub outcome: &'static str,
    pub ttfb_ms: u64,
    pub total_ms: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub tokens_estimated: bool,
    pub saved_usd: Option<f64>,
}

pub fn now_ms() -> u64 {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    d.map_or(0, |d| d.as_millis() as u64)
}

pub struct Logger {
    path: PathBuf,
}

impl Logger {
    pub fn new(path: &str) -> Self {
        Self { path: path.into() }
    }

    /// A failed write warns on stderr and never fails the request.
    pub fn write(&self, rec: &Record) {
        let result = serde_json::to_string(rec)
            .map_err(std::io::Error::other)
            .and_then(|line| {
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)?;
                writeln!(f, "{line}")
            });
        if let Err(e) = result {
            eprintln!("slm-router: log write failed: {e}");
        }
    }
}
