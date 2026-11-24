//! One upstream call, with the local model gated by the circuit breaker.

use super::AppState;
use crate::{
    routing::Target,
    upstream::{Api, Opened, Unreachable, open_stream, post_json},
};
use serde_json::Value;
use std::time::{Duration, Instant};

/// Calls `target`; a local behind an open circuit is skipped (`true`) and reported unreachable.
/// Only real local attempts are recorded; an attempt dropped by a client disconnect never is,
/// and the breaker's probe lease covers that.
pub async fn attempt(
    s: &AppState,
    body: &mut Value,
    api: Api,
    target: Target,
) -> (Result<Opened, Unreachable>, bool) {
    if target == Target::Local && !s.breaker.allow(Instant::now()) {
        return (Err(Unreachable("local circuit open".into())), true);
    }
    let result = call(s, body, api, target).await;
    if target == Target::Local {
        s.breaker.record(!local_failed(&result), Instant::now());
    }
    (result, false)
}

/// Unreachable or 5xx. A 4xx or a stream that dies later means local is alive.
pub fn local_failed(r: &Result<Opened, Unreachable>) -> bool {
    match r {
        Ok(Opened::Full(reply)) => reply.status >= 500,
        Ok(Opened::Stream(_)) => false,
        Err(_) => true,
    }
}

pub async fn call(
    s: &AppState,
    body: &mut Value,
    api: Api,
    target: Target,
) -> Result<Opened, Unreachable> {
    let (base, key, timeout, model) = match target {
        Target::Local => {
            let wait = Duration::from_millis(s.cfg.local.first_byte_timeout_ms);
            (&s.cfg.local.base_url, None, Some(wait), &s.cfg.local.model)
        }
        Target::Cloud => (
            &s.cfg.cloud.base_url,
            s.cfg.cloud.api_key.as_deref(),
            None,
            &s.cfg.cloud.model,
        ),
    };
    body["model"] = Value::from(model.as_str());
    if body["stream"] == true {
        open_stream(&s.http, base, api, key, body, timeout).await
    } else {
        post_json(&s.http, base, api, key, body, timeout)
            .await
            .map(Opened::Full)
    }
}
