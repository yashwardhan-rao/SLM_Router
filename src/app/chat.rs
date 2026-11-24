//! `POST /v1/chat/completions`, streaming and non-streaming.

use super::{
    AppState,
    policy::{effective_policy, route_error},
    stream,
    telemetry::{new_record, saved_usd, usage},
};
use crate::{
    errors::ApiError,
    routing::{Policy, Reason, Target, decide},
    schema::{ChatRequest, features},
    session::session_id,
    upstream::{Opened, Reply, Unreachable, open_stream, post_json},
};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, header::CONTENT_TYPE},
    response::{IntoResponse, Response},
};
use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

fn relay(r: Reply) -> Response {
    let status = StatusCode::from_u16(r.status).unwrap_or(StatusCode::BAD_GATEWAY);
    (status, [(CONTENT_TYPE, "application/json")], r.body).into_response()
}

pub async fn chat(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let mut req: ChatRequest = serde_json::from_slice(&body)
        .map_err(|e| ApiError::bad_request(format!("invalid request body: {e}")))?;
    let policy = effective_policy(s.cfg.server.policy, headers.get("x-router-policy"))?;
    let feats = features(&req);
    let started = Instant::now();
    let hdr = headers.get("x-session-id").and_then(|v| v.to_str().ok());
    let (sid, source) = session_id(hdr, &req);
    let decision = decide(&feats, &s.cfg.routing, policy, s.pins.get(&sid)).map_err(route_error)?;

    let seq = s.seq.fetch_add(1, Ordering::Relaxed);
    let mut rec = new_record(seq, sid.clone(), source, policy);
    rec.target = decision.target;
    rec.reason = decision.reason;
    rec.pinned = decision.reason == Reason::Pinned;
    rec.stream = req.stream == Some(true);

    let mut result = call(&s, &mut req, decision.target).await;
    if decision.target == Target::Local && local_failed(&result) {
        // Failover only for auto-routed requests; forced-local and local-only must not leak to cloud.
        // No byte has reached the client yet, so switching targets is still invisible to it.
        if decision.reason == Reason::Forced || policy == Policy::LocalOnly {
            rec.status = 503;
            rec.outcome = "upstream_error";
            rec.total_ms = started.elapsed().as_millis() as u64;
            s.logger.write(&rec);
            return Err(ApiError::upstream(
                StatusCode::SERVICE_UNAVAILABLE,
                "local model unavailable and cloud fallback is not allowed",
            )
            .code("local_unavailable"));
        }
        (rec.target, rec.reason, rec.failed_over, rec.pinned) =
            (Target::Cloud, Reason::LocalUnavailable, true, false);
        result = call(&s, &mut req, Target::Cloud).await;
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
            rec.total_ms = started.elapsed().as_millis() as u64;
            s.logger.write(&rec);
            Ok(relay(reply))
        }
        // The relay task owns the record and logs it when the stream ends.
        Ok(Opened::Stream(head)) => Ok(stream::relay(
            s,
            rec,
            started,
            head,
            feats.est_prompt_tokens,
        )),
        Err(e) => {
            rec.status = 502;
            rec.outcome = "upstream_error";
            rec.total_ms = started.elapsed().as_millis() as u64;
            s.logger.write(&rec);
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

async fn call(s: &AppState, req: &mut ChatRequest, target: Target) -> Result<Opened, Unreachable> {
    let (base, key, timeout) = match target {
        Target::Local => {
            req.model = s.cfg.local.model.clone();
            let wait = Duration::from_millis(s.cfg.local.first_byte_timeout_ms);
            (&s.cfg.local.base_url, None, Some(wait))
        }
        Target::Cloud => {
            req.model = s.cfg.cloud.model.clone();
            (&s.cfg.cloud.base_url, s.cfg.cloud.api_key.as_deref(), None)
        }
    };
    if req.stream == Some(true) {
        open_stream(&s.http, base, key, req, timeout).await
    } else {
        post_json(&s.http, base, key, req, timeout)
            .await
            .map(Opened::Full)
    }
}
