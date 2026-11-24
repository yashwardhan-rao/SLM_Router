//! Plain HTTP calls to OpenAI-compatible upstreams. No translation: bytes in, bytes out.

use bytes::Bytes;
use serde::Serialize;
use std::time::{Duration, Instant};

pub struct Reply {
    pub status: u16,
    pub body: Bytes,
    /// Milliseconds until response headers arrived.
    pub ttfb_ms: u64,
}

/// A 2xx event stream whose first chunk has arrived; the rest is still on `resp`.
pub struct StreamHead {
    pub status: u16,
    pub content_type: String,
    pub first: Bytes,
    pub resp: reqwest::Response,
    /// Milliseconds until the first body chunk arrived.
    pub ttfb_ms: u64,
}

pub enum Opened {
    /// A complete response: every non-stream reply, and error replies to stream requests.
    Full(Reply),
    Stream(StreamHead),
}

/// Connection failure or first-byte timeout: the upstream never produced a response.
#[derive(Debug)]
pub struct Unreachable(pub String);

fn unreachable_err(e: reqwest::Error) -> Unreachable {
    Unreachable(e.to_string())
}

fn request<T: Serialize>(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
    body: &T,
) -> reqwest::RequestBuilder {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let rb = client.post(url).json(body);
    match api_key {
        Some(k) => rb.bearer_auth(k),
        None => rb,
    }
}

async fn within<T, F: Future<Output = Result<T, Unreachable>>>(
    limit: Option<Duration>,
    fut: F,
) -> Result<T, Unreachable> {
    match limit {
        Some(d) => tokio::time::timeout(d, fut)
            .await
            .map_err(|_| Unreachable("first-byte timeout".into()))?,
        None => fut.await,
    }
}

pub async fn post_json<T: Serialize>(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
    body: &T,
    first_byte: Option<Duration>,
) -> Result<Reply, Unreachable> {
    let rb = request(client, base_url, api_key, body);
    let started = Instant::now();
    within(first_byte, async {
        let resp = rb.send().await.map_err(unreachable_err)?;
        let ttfb_ms = started.elapsed().as_millis() as u64;
        let status = resp.status().as_u16();
        let body = resp.bytes().await.map_err(unreachable_err)?;
        Ok(Reply {
            status,
            body,
            ttfb_ms,
        })
    })
    .await
}

/// Send a `stream: true` request and wait for the first body chunk (within `first_byte`).
/// Nothing has reached the client yet, so the caller may still fail over.
pub async fn open_stream<T: Serialize>(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
    body: &T,
    first_byte: Option<Duration>,
) -> Result<Opened, Unreachable> {
    let rb = request(client, base_url, api_key, body);
    let started = Instant::now();
    within(first_byte, async {
        let mut resp = rb.send().await.map_err(unreachable_err)?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let body = resp.bytes().await.map_err(unreachable_err)?;
            let ttfb_ms = started.elapsed().as_millis() as u64;
            return Ok(Opened::Full(Reply {
                status,
                body,
                ttfb_ms,
            }));
        }
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("text/event-stream")
            .to_string();
        let first = resp
            .chunk()
            .await
            .map_err(unreachable_err)?
            .unwrap_or_default();
        let ttfb_ms = started.elapsed().as_millis() as u64;
        Ok(Opened::Stream(StreamHead {
            status,
            content_type,
            first,
            resp,
            ttfb_ms,
        }))
    })
    .await
}
