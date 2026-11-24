//! In-memory session pins: keep a conversation on one target so the local KV cache stays warm.

use crate::routing::Target;
use crate::schema::{ChatRequest, text_of};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::time::Duration;
use tokio::time::Instant;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionSource {
    Header,
    Fingerprint,
}

/// `X-Session-Id` when given, else a hash of the system prompt plus the first user message.
pub fn session_id(header: Option<&str>, req: &ChatRequest) -> (String, SessionSource) {
    if let Some(h) = header.filter(|h| !h.is_empty()) {
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

pub struct PinStore {
    ttl: Duration,
    max: usize,
    pins: Mutex<HashMap<String, (Target, Instant)>>,
}

impl PinStore {
    pub fn new(ttl: Duration, max: usize) -> Self {
        Self {
            ttl,
            max,
            pins: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, id: &str) -> Option<Target> {
        let mut pins = self.pins.lock().unwrap_or_else(|e| e.into_inner());
        match pins.get(id) {
            Some((t, at)) if at.elapsed() < self.ttl => Some(*t),
            Some(_) => {
                pins.remove(id);
                None
            }
            None => None,
        }
    }

    /// Record a request on `id`: refreshes the TTL, allows local -> cloud, never cloud -> local.
    pub fn set(&self, id: &str, target: Target) {
        let mut pins = self.pins.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if let Some((cur, at)) = pins.get_mut(id)
            && at.elapsed() < self.ttl
        {
            if target == Target::Cloud {
                *cur = Target::Cloud;
            }
            *at = now;
            return;
        }
        if pins.len() >= self.max && !pins.contains_key(id) {
            // shortcut: O(n) scan for the LRU entry; fine at 10k pins, use a linked map beyond that.
            let oldest = pins
                .iter()
                .min_by_key(|(_, (_, at))| *at)
                .map(|(k, _)| k.clone());
            if let Some(k) = oldest {
                pins.remove(&k);
            }
        }
        pins.insert(id.to_string(), (target, now));
    }
}
