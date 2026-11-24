//! Relay an upstream event stream to the client, then write its log record.

use super::{AppState, sse::SseScan, telemetry::saved_usd};
use crate::{logging::Record, upstream::StreamHead};
use axum::{
    body::{Body, Bytes},
    http::{HeaderValue, StatusCode, header},
    response::Response,
};
use std::{convert::Infallible, sync::Arc, time::Instant};
use tokio::sync::mpsc::{self, Sender};
use tokio_stream::wrappers::ReceiverStream;

const INTERRUPTED: &str = concat!(
    r#"data: {"error":{"message":"upstream connection lost mid-stream","#,
    r#""type":"upstream_error","code":"stream_interrupted"}}"#,
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
) -> Response {
    let (status, ctype) = (head.status, head.content_type.clone());
    (rec.status, rec.ttfb_ms) = (head.status, head.ttfb_ms);
    let (tx, rx) = mpsc::channel::<Result<Bytes, Infallible>>(16);
    tokio::spawn(async move {
        let (outcome, scan) = pump(head, &tx).await;
        let (p, c, est) = scan.tokens(est_prompt);
        rec.outcome = outcome;
        (
            rec.prompt_tokens,
            rec.completion_tokens,
            rec.tokens_estimated,
        ) = (p, c, est);
        rec.saved_usd = saved_usd(rec.target, s.cfg.pricing, p, c);
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
async fn pump(head: StreamHead, tx: &Sender<Result<Bytes, Infallible>>) -> (&'static str, SseScan) {
    let mut scan = SseScan::default();
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
                scan.feed(&b);
                if tx.send(Ok(b)).await.is_err() {
                    return ("client_disconnect", scan);
                }
            }
            Ok(None) => return ("ok", scan),
            Err(_) => {
                let _ = tx
                    .send(Ok(Bytes::from_static(INTERRUPTED.as_bytes())))
                    .await;
                return ("stream_error", scan);
            }
        }
    }
}
