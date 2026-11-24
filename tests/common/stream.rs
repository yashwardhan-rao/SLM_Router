use super::user;
use serde_json::{Value, json};

pub const HELLO: &str = r#"{"choices":[{"delta":{"content":"hello world!"}}]}"#;
pub const USAGE: &str =
    r#"{"choices":[],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}}"#;

pub fn stream_req() -> Value {
    let mut r = user("hi");
    r["stream"] = json!(true);
    r
}

pub fn forced_cloud() -> Value {
    let mut r = stream_req();
    r["model"] = json!("slm-router/cloud");
    r
}

/// Status, content-type and the whole body of a streamed response.
pub async fn fetch(base: &str, body: &Value, headers: &[(&str, &str)]) -> (u16, String, String) {
    let mut r = reqwest::Client::new()
        .post(format!("{base}/v1/chat/completions"))
        .json(body);
    for (k, v) in headers {
        r = r.header(*k, *v);
    }
    let resp = r.send().await.unwrap();
    let status = resp.status().as_u16();
    let ctype = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    (status, ctype, resp.text().await.unwrap_or_default())
}

pub fn sse(payloads: &[&str]) -> String {
    let mut s: String = payloads.iter().map(|p| format!("data: {p}\n\n")).collect();
    s.push_str("data: [DONE]\n\n");
    s
}
