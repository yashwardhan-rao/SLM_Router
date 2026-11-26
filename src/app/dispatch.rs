//! Shared tail of both endpoints: route, call, fail over, pin, relay, log.

use super::{
    AppState,
    guard::Guard,
    policy::{effective_policy, route_error, unsupported},
    stream,
    target::{attempt, call, local_failed},
    telemetry::{new_record, saved_usd, usage},
};
use crate::{
    errors::ApiError,
    routing::{Features, Policy, Reason, Target, decide},
    session::SessionSource,
    upstream::{Api, Opened, Reply},
};
use axum::{
    http::{HeaderMap, StatusCode, header::CONTENT_TYPE},
    response::{IntoResponse, Response},
};
use serde_json::Value;
use std::{
    sync::{Arc, atomic::Ordering},
    time::Instant,
};

/// A validated request, ready to route. `body` is forwarded as is, apart from `model`.
pub struct Job {
    pub api: Api,
    pub body: Value,
    pub feats: Features,
    pub sid: String,
    pub source: SessionSource,
    /// The router asked for the stream's usage chunk; the client did not, so it is hidden.
    pub hide_usage: bool,
}

fn relay(r: Reply) -> Response {
    let status = StatusCode::from_u16(r.status).unwrap_or(StatusCode::BAD_GATEWAY);
    (status, [(CONTENT_TYPE, "application/json")], r.body).into_response()
}

pub async fn serve(s: Arc<AppState>, headers: &HeaderMap, job: Job) -> Result<Response, ApiError> {
    let Job {
        api,
        mut body,
        feats,
        sid,
        source,
        hide_usage,
    } = job;
    let policy = effective_policy(s.cfg.server.policy, headers.get("x-router-policy"))?;
    let started = Instant::now();
    let decision = decide(&feats, &s.cfg.routing, policy, s.pins.get(&sid)).map_err(route_error)?;

    let seq = s.seq.fetch_add(1, Ordering::Relaxed);
    let mut rec = Guard::new(
        s.clone(),
        new_record(seq, sid.clone(), source, policy),
        started,
    );
    rec.target = decision.target;
    rec.reason = decision.reason;
    rec.pinned = decision.reason == Reason::Pinned;
    rec.stream = body["stream"] == true;

    let (mut result, skipped) = attempt(&s, &mut body, api, decision.target).await;
    if decision.target == Target::Local && local_failed(&result) {
        // Failover only for auto-routed requests; forced-local and local-only must not leak to cloud.
        // No byte has reached the client yet, so switching targets is still invisible to it.
        if decision.reason == Reason::Forced || policy == Policy::LocalOnly {
            rec.status = 503;
            rec.outcome = "upstream_error";
            rec.log();
            return Err(ApiError::upstream(
                StatusCode::SERVICE_UNAVAILABLE,
                "local model unavailable and cloud fallback is not allowed",
            )
            .code("local_unavailable"));
        }
        let reason = match skipped {
            true => Reason::CircuitOpen,
            false => Reason::LocalUnavailable,
        };
        (rec.target, rec.reason, rec.failed_over, rec.pinned) =
            (Target::Cloud, reason, true, false);
        result = call(&s, &mut body, api, Target::Cloud).await;
    }
    // Checked on the final reply only, so a missing route is never retried elsewhere,
    // and before pinning, so a target that cannot serve the endpoint is not made sticky.
    if let Ok(Opened::Full(reply)) = &result
        && let Some(err) = unsupported(api, rec.target, reply)
    {
        (rec.status, rec.ttfb_ms, rec.outcome) = (501, reply.ttfb_ms, "upstream_error");
        rec.log();
        return Err(err);
    }
    // Failovers do not pin: local is retried (or skipped by the breaker) on the next turn.
    let pin = !matches!(
        rec.reason,
        Reason::Forced | Reason::Policy | Reason::LocalUnavailable | Reason::CircuitOpen
    );
    if result.is_ok() && pin {
        s.pins.set(&sid, rec.target);
    }
    match result {
        Ok(Opened::Full(reply)) => {
            let ok = reply.status < 400;
            let (p, c, est) = match ok {
                true => usage(&reply.body, feats.est_prompt_tokens),
                false => (0, 0, false),
            };
            rec.status = reply.status;
            rec.ttfb_ms = reply.ttfb_ms;
            rec.outcome = if ok { "ok" } else { "upstream_error" };
            (
                rec.prompt_tokens,
                rec.completion_tokens,
                rec.tokens_estimated,
            ) = (p, c, est);
            rec.saved_usd = saved_usd(rec.target, s.cfg.pricing, p, c);
            rec.log();
            Ok(relay(reply))
        }
        // The relay task owns the record and logs it when the stream ends.
        Ok(Opened::Stream(head)) => Ok(stream::relay(
            s,
            rec.into_record(),
            started,
            head,
            feats.est_prompt_tokens,
            api,
            hide_usage,
        )),
        Err(e) => {
            rec.status = 502;
            rec.outcome = "upstream_error";
            rec.log();
            Err(ApiError::upstream(
                StatusCode::BAD_GATEWAY,
                format!("upstream unreachable: {}", e.0),
            ))
        }
    }
}
