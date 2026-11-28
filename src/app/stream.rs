//! Relay an upstream event stream to the client, then write its log record.

use super::{AppState, sse::SseScan, telemetry::saved_usd};
use crate::{
    logging::Record,
    upstream::{Api, StreamHead},
};
use axum::{
    body::{Body, Bytes},
    http::{HeaderValue, StatusCode, header},
    response::Response,
};
use std::{convert::Infallible, sync::Arc, time::Instant};
use tokio::sync::mpsc::{self, Sender};
use tokio_stream::wrappers::ReceiverStream;

const INTERRUPTED_CHAT: &str = concat!(
    r#"data: {"error":{"message":"upstream connection lost mid-stream","#,
    r#""type":"upstream_error","code":"stream_interrupted"}}"#,
    "\n\n"
);

/// Responses-API clients dispatch on the `event:` name, so they get a native `error` event.
const INTERRUPTED_RESPONSES: &str = concat!(
    "event: error\n",
    r#"data: {"type":"error","code":"stream_interrupted","#,
    r#""message":"upstream connection lost mid-stream","param":null}"#,
    "\n\n"
);

/// Start relaying `head` and return the response immediately. The log record is written when
/// the stream ends, however it ends.
pub fn relay(
    s: Arc<AppState>,
    mut rec: Record,
    started: Instant,
    head: StreamHead,
    est_prompt: usize,
    api: Api,
    hide_usage: bool,
) -> Response {
    let (status, ctype) = (head.status, head.content_type.clone());
    (rec.status, rec.ttfb_ms) = (head.status, head.ttfb_ms);
    let (tx, rx) = mpsc::channel::<Result<Bytes, Infallible>>(16);
    let interrupted = match api {
        Api::Chat => INTERRUPTED_CHAT,
        Api::Responses => INTERRUPTED_RESPONSES,
    };
    tokio::spawn(async move {
        let scan = match hide_usage {
            true => SseScan::hiding_usage(),
            false => SseScan::default(),
        };
        let (outcome, scan) = pump(head, &tx, interrupted, scan).await;
        let (p, c, est) = scan.tokens(est_prompt);
        rec.outcome = outcome;
        (
            rec.prompt_tokens,
            rec.completion_tokens,
            rec.tokens_estimated,
        ) = (p, c, est);
        // Tier names are unique, so the record's name finds the tier that answered.
        let tier = s.ladder.iter().position(|t| t.name == rec.tier);
        rec.saved_usd = tier.and_then(|i| saved_usd(&s.ladder, i, p, c));
        rec.total_ms = started.elapsed().as_millis() as u64;
        s.logger.write(&rec);
    });
    let mut resp = Response::new(Body::from_stream(ReceiverStream::new(rx)));
    *resp.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
    let h = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&ctype) {
        h.insert(header::CONTENT_TYPE, v);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    resp
}

/// Forward chunks until the stream ends, the upstream dies or the client goes away.
/// Returning drops the upstream response, which cancels the upstream request.
async fn pump(
    head: StreamHead,
    tx: &Sender<Result<Bytes, Infallible>>,
    interrupted: &'static str,
    mut scan: SseScan,
) -> (&'static str, SseScan) {
    let mut resp = head.resp;
    let mut next = Some(head.first);
    loop {
        let chunk = match next.take() {
            Some(first) => Ok(Some(first)),
            None => tokio::select! {
                c = resp.chunk() => c,
                () = tx.closed() => return ("client_disconnect", scan),
            },
        };
        match chunk {
            Ok(Some(b)) => {
                let kept = scan.feed(&b);
                // A filtering scan forwards only what it kept; a plain one forwards `b` untouched.
                let out = if scan.filters() { Bytes::from(kept) } else { b };
                if !out.is_empty() && tx.send(Ok(out)).await.is_err() {
                    return ("client_disconnect", scan);
                }
            }
            Ok(None) => {
                send_tail(tx, &mut scan).await;
                return ("ok", scan);
            }
            Err(_) => {
                send_tail(tx, &mut scan).await;
                let _ = tx
                    .send(Ok(Bytes::from_static(interrupted.as_bytes())))
                    .await;
                return ("stream_error", scan);
            }
        }
    }
}

/// Forward an unterminated last line the filter was still holding.
async fn send_tail(tx: &Sender<Result<Bytes, Infallible>>, scan: &mut SseScan) {
    let tail = scan.tail();
    if !tail.is_empty() {
        let _ = tx.send(Ok(Bytes::from(tail))).await;
    }
}
