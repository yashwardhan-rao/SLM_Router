//! Shared tail of both endpoints: route, call, fail over, pin, relay, log.

use super::{
    AppState,
    guard::Guard,
    policy::{effective_policy, route_error},
    stream,
    telemetry::{new_record, saved_usd, usage},
};
use crate::{
    errors::ApiError,
    routing::{Features, Policy, Reason, Target, decide},
    session::SessionSource,
    upstream::{Api, Opened, Reply, Unreachable, open_stream, post_json},
};
use axum::{
    http::{HeaderMap, StatusCode, header::CONTENT_TYPE},
    response::{IntoResponse, Response},
};
use serde_json::Value;
use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

/// A validated request, ready to route. `body` is forwarded as is, apart from `model`.
pub struct Job {
    pub api: Api,
    pub body: Value,
    pub feats: Features,
    pub sid: String,
    pub source: SessionSource,
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

    let mut result = call(&s, &mut body, api, decision.target).await;
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
        (rec.target, rec.reason, rec.failed_over, rec.pinned) =
            (Target::Cloud, Reason::LocalUnavailable, true, false);
        result = call(&s, &mut body, api, Target::Cloud).await;
    }
    if result.is_ok() && !matches!(decision.reason, Reason::Forced | Reason::Policy) {
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

fn local_failed(r: &Result<Opened, Unreachable>) -> bool {
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
