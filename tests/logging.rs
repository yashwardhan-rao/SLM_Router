mod common;
use common::*;
use serde_json::json;

#[tokio::test]
async fn one_metadata_line_per_request_with_tokens_latency_and_savings() {
    let r = rig(Mode::Ok, Mode::Ok, |_| {}).await;
    post_chat(&r.base, &user("hi"), &[("x-session-id", "s1")]).await;
    let lines = log_lines(&r.dir.log());
    assert_eq!(lines.len(), 1);
    let l = &lines[0];
    assert_eq!(
        (l["v"].as_u64(), l["target"].as_str(), l["reason"].as_str()),
        (Some(1), Some("local"), Some("default"))
    );
    assert_eq!(
        (l["session_id"].as_str(), l["session_source"].as_str()),
        (Some("s1"), Some("header"))
    );
    assert_eq!(
        (
            l["status"].as_u64(),
            l["outcome"].as_str(),
            l["failed_over"].as_bool()
        ),
        (Some(200), Some("ok"), Some(false))
    );
    assert_eq!(
        (
            l["prompt_tokens"].as_u64(),
            l["completion_tokens"].as_u64(),
            l["tokens_estimated"].as_bool()
        ),
        (Some(10), Some(5), Some(false))
    );
    // What cloud would have cost: 10 * $3/M + 5 * $15/M.
    assert!((l["saved_usd"].as_f64().unwrap() - 0.000105).abs() < 1e-12);
    assert!(
        l["total_ms"].is_u64()
            && l["ttfb_ms"].is_u64()
            && l["request_id"].is_string()
            && l["ts"].is_u64()
    );
}

#[tokio::test]
async fn failover_is_logged_and_saves_nothing() {
    let r = rig(Mode::Status(503, "{}"), Mode::Ok, |_| {}).await;
    post_chat(&r.base, &user("hi"), &[]).await;
    let l = &log_lines(&r.dir.log())[0];
    assert_eq!(
        (
            l["target"].as_str(),
            l["reason"].as_str(),
            l["failed_over"].as_bool()
        ),
        (Some("cloud"), Some("local_unavailable"), Some(true))
    );
    assert_eq!(l["saved_usd"].as_f64(), Some(0.0));
    assert_eq!(l["session_source"], "fingerprint");
}

#[tokio::test]
async fn pinned_requests_are_flagged() {
    let r = rig(Mode::Ok, Mode::Ok, |_| {}).await;
    let h = [("x-session-id", "s")];
    post_chat(&r.base, &user("a"), &h).await;
    post_chat(&r.base, &user("b"), &h).await;
    let lines = log_lines(&r.dir.log());
    assert_eq!(
        (lines[0]["pinned"].as_bool(), lines[1]["pinned"].as_bool()),
        (Some(false), Some(true))
    );
    assert_eq!(lines[1]["reason"], "pinned");
}

#[tokio::test]
async fn upstream_errors_are_logged_with_their_status() {
    let r = rig(Mode::Ok, Mode::Status(429, "{}"), |_| {}).await;
    let mut req = user("hi");
    req["model"] = json!("slm-router/cloud");
    post_chat(&r.base, &req, &[]).await;
    let l = &log_lines(&r.dir.log())[0];
    assert_eq!(
        (l["status"].as_u64(), l["outcome"].as_str()),
        (Some(429), Some("upstream_error"))
    );
}

#[tokio::test]
async fn saved_usd_is_null_without_pricing() {
    let r = rig(Mode::Ok, Mode::Ok, |c| c.pricing = None).await;
    post_chat(&r.base, &user("hi"), &[]).await;
    assert!(log_lines(&r.dir.log())[0]["saved_usd"].is_null());
}

#[tokio::test]
async fn log_never_contains_prompt_text_or_keys() {
    let r = rig(Mode::Ok, Mode::Ok, |_| {}).await;
    post_chat(
        &r.base,
        &user("TOP-SECRET-PROMPT"),
        &[("authorization", "Bearer client-key")],
    )
    .await;
    let raw = std::fs::read_to_string(r.dir.log()).unwrap();
    for needle in ["TOP-SECRET-PROMPT", "sk-cloud", "client-key", "hello"] {
        assert!(!raw.contains(needle), "log leaked {needle}");
    }
}

#[tokio::test]
async fn a_log_write_failure_does_not_fail_the_request() {
    let dir = TempDir::new();
    // The log path is a directory, so every append fails.
    let r = rig(Mode::Ok, Mode::Ok, |c| {
        c.log.path = dir.0.to_str().unwrap().into()
    })
    .await;
    let (status, _) = post_chat(&r.base, &user("hi"), &[]).await;
    assert_eq!(status, 200);
}
