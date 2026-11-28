//! One upstream call to a tier, gated by that tier's circuit breaker.

use super::AppState;
use crate::upstream::{Api, Opened, Unreachable, open_stream, post_json};
use serde_json::Value;
use std::time::Instant;

/// Calls ladder tier `tier`; one behind an open circuit is skipped (`true`) and reported
/// unreachable. An attempt dropped by a client disconnect is never recorded; the breaker's probe
/// lease covers that.
pub async fn attempt(
    s: &AppState,
    body: &mut Value,
    api: Api,
    tier: usize,
) -> (Result<Opened, Unreachable>, bool) {
    let breaker = &s.breakers[tier];
    if !breaker.allow(Instant::now()) {
        return (Err(Unreachable("circuit open".into())), true);
    }
    let result = call(s, body, api, tier).await;
    breaker.record(!failed(&result), Instant::now());
    (result, false)
}

/// Unreachable or 5xx. A 4xx or a stream that dies later means the tier is alive.
pub fn failed(r: &Result<Opened, Unreachable>) -> bool {
    match r {
        Ok(Opened::Full(reply)) => reply.status >= 500,
        Ok(Opened::Stream(_)) => false,
        Err(_) => true,
    }
}

async fn call(
    s: &AppState,
    body: &mut Value,
    api: Api,
    tier: usize,
) -> Result<Opened, Unreachable> {
    let t = &s.ladder[tier];
    let (key, timeout) = (t.api_key.as_deref(), t.first_byte_timeout());
    body["model"] = Value::from(t.model.as_str());
    if body["stream"] == true {
        open_stream(&s.http, &t.base_url, api, key, body, timeout).await
    } else {
        post_json(&s.http, &t.base_url, api, key, body, timeout)
            .await
            .map(Opened::Full)
    }
}
