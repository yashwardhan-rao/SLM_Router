//! In-memory session pins: keep a conversation on one tier so its KV cache stays warm.

use crate::schema::{ChatRequest, text_of};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::time::Duration;
use tokio::time::Instant;

#[cfg(test)]
mod tests;

/// Longest accepted `X-Session-Id`, in bytes.
const MAX_SESSION_ID: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionSource {
    Header,
    Fingerprint,
}

/// `X-Session-Id` when given, else a hash of the system prompt plus the first user message.
pub fn session_id(header: Option<&str>, req: &ChatRequest) -> (String, SessionSource) {
    // Long ids would bloat the pin table and the log; those clients get a fingerprint instead.
    if let Some(h) = header.filter(|h| !h.is_empty() && h.len() <= MAX_SESSION_ID) {
        return (h.to_string(), SessionSource::Header);
    }
    let first = |role: &str| {
        let m = req.messages.iter().find(|m| m.role == role);
        m.map(|m| text_of(&m.content)).unwrap_or_default()
    };
    // shortcut: SipHash is one-way enough for an in-memory key; swap for SHA-256 if ids are ever persisted.
    let mut h = DefaultHasher::new();
    (first("system"), first("user")).hash(&mut h);
    (
        format!("fp-{:016x}", h.finish()),
        SessionSource::Fingerprint,
    )
}

/// Pin map plus a recency queue; refreshed ids are re-queued and their old queue entries go stale.
#[derive(Default)]
struct Pins {
    map: HashMap<String, (usize, Instant, u64)>,
    order: VecDeque<(String, u64)>,
    seq: u64,
}

pub struct PinStore {
    ttl: Duration,
    max: usize,
    pins: Mutex<Pins>,
}

impl PinStore {
    pub fn new(ttl: Duration, max: usize) -> Self {
        Self {
            ttl,
            max,
            pins: Mutex::default(),
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.pins.lock().unwrap().map.len()
    }

    pub fn get(&self, id: &str) -> Option<usize> {
        let mut p = self.pins.lock().unwrap_or_else(|e| e.into_inner());
        match p.map.get(id) {
            Some((t, at, _)) if at.elapsed() < self.ttl => Some(*t),
            Some(_) => {
                p.map.remove(id);
                None
            }
            None => None,
        }
    }

    /// Record a request on `id` at ladder index `tier`: refreshes the TTL; a pin only moves up.
    /// Eviction and refresh are amortised O(1).
    pub fn set(&self, id: &str, tier: usize) {
        let mut guard = self.pins.lock().unwrap_or_else(|e| e.into_inner());
        let p = &mut *guard;
        p.seq += 1;
        let (now, seq) = (Instant::now(), p.seq);
        match p.map.get_mut(id) {
            Some((cur, at, s)) if at.elapsed() < self.ttl => {
                *cur = (*cur).max(tier);
                (*at, *s) = (now, seq);
            }
            existing => {
                let known = existing.is_some();
                if p.map.len() >= self.max && !known {
                    // Skip queue entries made stale by a refresh or an expiry.
                    while let Some((k, s)) = p.order.pop_front() {
                        if p.map.get(&k).is_some_and(|e| e.2 == s) {
                            p.map.remove(&k);
                            break;
                        }
                    }
                }
                p.map.insert(id.to_string(), (tier, now, seq));
            }
        }
        p.order.push_back((id.to_string(), seq));
        if p.order.len() > 2 * self.max + 16 {
            let map = &p.map;
            p.order
                .retain(|(k, s)| map.get(k).is_some_and(|e| e.2 == *s));
        }
    }
}
