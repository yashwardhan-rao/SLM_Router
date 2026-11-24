//! Plain HTTP calls to OpenAI-compatible upstreams. No translation: bytes in, bytes out.

use bytes::Bytes;
use serde::Serialize;
use std::time::Duration;

pub struct Reply {
    pub status: u16,
    pub body: Bytes,
    /// Milliseconds until response headers arrived.
    pub ttfb_ms: u64,
}

/// Connection failure or first-byte timeout: the upstream never produced a response.
#[derive(Debug)]
pub struct Unreachable(pub String);

pub async fn post_json<T: Serialize>(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
    body: &T,
    first_byte: Option<Duration>,
) -> Result<Reply, Unreachable> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let mut rb = client.post(url).json(body);
    if let Some(k) = api_key {
        rb = rb.bearer_auth(k);
    }
    let started = std::time::Instant::now();
    let sent = match first_byte {
        Some(d) => tokio::time::timeout(d, rb.send())
            .await
            .map_err(|_| Unreachable("first-byte timeout".into()))?,
        None => rb.send().await,
    };
    let resp = sent.map_err(|e| Unreachable(e.to_string()))?;
    let ttfb_ms = started.elapsed().as_millis() as u64;
    let status = resp.status().as_u16();
    let body = resp.bytes().await.map_err(|e| Unreachable(e.to_string()))?;
    Ok(Reply {
        status,
        body,
        ttfb_ms,
    })
}
