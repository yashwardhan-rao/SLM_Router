//! Shared tail of both endpoints: route, call, fail over, pin, relay, log.

use super::{
    AppState,
    guard::Guard,
    policy::{effective_policy, route_error, unsupported},
    stream,
    target::{attempt, failed},
    telemetry::{new_record, saved_usd, usage},
};
use crate::{
    errors::ApiError,
    routing::{Features, Forced, Policy, Reason, Target, classifier_score, decide, next_up},
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
        mut feats,
        sid,
        source,
        hide_usage,
    } = job;
    let policy = effective_policy(s.cfg.server.policy, headers.get("x-router-policy"))?;
    // A tier's own name wins over the `local` / `cloud` aliases.
    let named = body["model"]
        .as_str()
        .and_then(|m| m.strip_prefix("slm-router/"));
    if let Some(i) = named.and_then(|n| s.ladder.iter().position(|t| t.name == n)) {
        feats.forced = Forced::Tier(i);
    }
    let started = Instant::now();
    let decision = decide(&feats, &s.cfg.routing, policy, s.pins.get(&sid)).map_err(route_error)?;

    let seq = s.seq.fetch_add(1, Ordering::Relaxed);
    let mut rec = Guard::new(
        s.clone(),
        new_record(seq, sid.clone(), source, policy),
        started,
    );
    // Set before any await, so a client that hangs up is logged against the tier being tried.
    (rec.target, rec.tier) = (decision.target, s.ladder[decision.tier].name.clone());
    rec.reason = decision.reason;
    rec.pinned = decision.reason == Reason::Pinned;
    rec.stream = body["stream"] == true;
    rec.classifier_score =
        classifier_score(&feats, &s.cfg.routing).map(|p| (f64::from(p) * 1000.0).round() / 1000.0);

    let specs = s.cfg.routing.specs();
    let mut tier = decision.tier;
    let (mut result, mut skipped) = attempt(&s, &mut body, api, tier).await;
    // Failover only for routed requests: a forced tier must not leak elsewhere. No byte has
    // reached the client yet, so switching tiers is still invisible to it. Up the ladder only.
    while failed(&result) && decision.reason != Reason::Forced {
        let Some(next) = next_up(&specs, tier, &feats, policy) else {
            break;
        };
        if !rec.failed_over {
            // The first hop's cause: why the tier routing chose did not answer.
            rec.reason = match (skipped, specs[tier].local) {
                (true, _) => Reason::CircuitOpen,
                (false, true) => Reason::LocalUnavailable,
                (false, false) => Reason::TierUnavailable,
            };
            (rec.failed_over, rec.pinned) = (true, false);
        }
        tier = next;
        (rec.target, rec.tier) = (target_of(specs[tier].local), s.ladder[tier].name.clone());
        (result, skipped) = attempt(&s, &mut body, api, tier).await;
    }
    let local = specs[tier].local;
    if failed(&result) && (skipped || local) {
        // A remote tier's own 5xx or 502 is relayed below; a dead local or open circuit is a 503.
        rec.status = 503;
        rec.outcome = "upstream_error";
        rec.log();
        let (code, msg) = match local {
            true => (
                "local_unavailable",
                "local model unavailable and no fallback is allowed",
            ),
            false => (
                "circuit_open",
                "upstream circuit open and no fallback is allowed",
            ),
        };
        return Err(ApiError::upstream(StatusCode::SERVICE_UNAVAILABLE, msg).code(code));
    }
    // Checked on the final reply only, so a missing route is never retried elsewhere,
    // and before pinning, so a tier that cannot serve the endpoint is not made sticky.
    if let Ok(Opened::Full(reply)) = &result
        && let Some(err) = unsupported(api, &s.ladder[tier].name, reply)
    {
        (rec.status, rec.ttfb_ms, rec.outcome) = (501, reply.ttfb_ms, "upstream_error");
        rec.log();
        return Err(err);
    }
    // Only auto routing pins. Failovers do not: the cheaper tier is retried (or skipped by its
    // breaker) next turn.
    let pin = policy == Policy::Auto && !(rec.failed_over || rec.reason == Reason::Forced);
    if result.is_ok() && pin {
        s.pins.set(&sid, tier);
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
            rec.saved_usd = saved_usd(&s.ladder, tier, p, c);
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

fn target_of(local: bool) -> Target {
    if local { Target::Local } else { Target::Cloud }
}
