//! The v1 log schema is frozen at 1.0.0: any added, removed or renamed field fails here.

mod common;
use common::*;
use serde_json::{Value, json};

const V1_FIELDS: [&str; 21] = [
    "v",
    "ts",
    "request_id",
    "session_id",
    "session_source",
    "policy",
    "target",
    "tier",
    "reason",
    "failed_over",
    "pinned",
    "stream",
    "status",
    "outcome",
    "ttfb_ms",
    "total_ms",
    "prompt_tokens",
    "completion_tokens",
    "tokens_estimated",
    "saved_usd",
    "classifier_score",
];

fn keys(line: &Value) -> Vec<String> {
    let mut k: Vec<String> = line.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

fn want() -> Vec<String> {
    let mut k: Vec<String> = V1_FIELDS.iter().map(|s| s.to_string()).collect();
    k.sort();
    k
}

#[tokio::test]
async fn chat_and_responses_lines_carry_exactly_the_v1_fields() {
    let r = rig(Mode::Ok, Mode::Ok, |_| {}).await;
    post_chat(&r.base, &user("hi"), &[]).await;
    let status = reqwest::Client::new()
        .post(format!("{}/v1/responses", r.base))
        .json(&json!({"model": "slm-router/auto", "input": "hi"}))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, 200);
    let lines = log_lines(&r.dir.log());
    assert_eq!(lines.len(), 2);
    for l in &lines {
        assert_eq!(keys(l), want(), "{l}");
        assert_eq!(l["v"], 1);
    }
}

#[tokio::test]
async fn an_error_line_carries_the_same_fields() {
    let r = rig(Mode::Status(429, "slow down"), Mode::Ok, |_| {}).await;
    let (status, _) = post_chat(
        &r.base,
        &json!({"model": "slm-router/local", "messages": [{"role": "user", "content": "hi"}]}),
        &[],
    )
    .await;
    assert_eq!(status, 429);
    let lines = log_lines(&r.dir.log());
    assert_eq!(keys(&lines[0]), want());
    assert_eq!(lines[0]["outcome"], "upstream_error");
}
