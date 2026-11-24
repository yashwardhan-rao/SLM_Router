mod common;
use common::*;
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn a_3mb_vision_payload_is_accepted_and_routed_to_cloud() {
    let r = rig(Mode::Ok, Mode::Ok, |_| {}).await;
    let image = format!("data:image/png;base64,{}", "A".repeat(3 * 1024 * 1024));
    let mut req = user("describe");
    req["messages"][0]["content"] = json!([{"type": "text", "text": "describe"}, {"type": "image_url", "image_url": {"url": image}}]);
    let (status, body) = post_chat(&r.base, &req, &[]).await;
    assert_eq!((status, body["model"].as_str()), (200, Some("cloud")));
    assert_eq!(r.cloud.count(), 1);
}

#[tokio::test]
async fn bodies_over_the_configured_limit_get_an_openai_style_413() {
    let r = rig(Mode::Ok, Mode::Ok, |c| c.server.max_body_bytes = 1_000).await;
    let (status, body) = post_chat(&r.base, &user(&"x".repeat(5_000)), &[]).await;
    assert_eq!(status, 413);
    assert!(
        body["error"]["message"].is_string() && body["error"]["type"].is_string(),
        "{body}"
    );
    assert_eq!(r.local.count() + r.cloud.count(), 0);
}

#[tokio::test]
async fn session_ids_over_128_bytes_fall_back_to_the_fingerprint() {
    let r = rig(Mode::Ok, Mode::Ok, |_| {}).await;
    let long = "s".repeat(129);
    let ok = "s".repeat(128);
    post_chat(&r.base, &user("hi"), &[("x-session-id", &long)]).await;
    post_chat(&r.base, &user("hi"), &[("x-session-id", &ok)]).await;
    let lines = log_lines_wait(&r.dir.log(), 2).await;
    assert_eq!(lines[0]["session_source"], "fingerprint");
    assert!(lines[0]["session_id"].as_str().unwrap().starts_with("fp-"));
    assert_eq!(
        (
            lines[1]["session_source"].as_str(),
            lines[1]["session_id"].as_str()
        ),
        (Some("header"), Some(ok.as_str()))
    );
}

#[tokio::test]
async fn fifty_concurrent_requests_write_fifty_valid_log_lines() {
    let r = rig(Mode::Ok, Mode::Ok, |_| {}).await;
    let mut set = tokio::task::JoinSet::new();
    for i in 0..50 {
        let base = r.base.clone();
        set.spawn(async move {
            let id = format!("s{i}");
            post_chat(&base, &user("hi"), &[("x-session-id", &id)])
                .await
                .0
        });
    }
    while let Some(status) = set.join_next().await {
        assert_eq!(status.unwrap(), 200);
    }
    // `log_lines` panics on any line that is not valid JSON.
    let lines = log_lines_wait(&r.dir.log(), 50).await;
    let ids: std::collections::HashSet<_> = lines
        .iter()
        .map(|l| l["session_id"].as_str().unwrap())
        .collect();
    assert_eq!((lines.len(), ids.len()), (50, 50));
}

#[tokio::test]
async fn a_client_that_hangs_up_before_the_first_byte_is_still_logged() {
    let r = rig(Mode::Hang, Mode::Ok, |c| {
        c.local.first_byte_timeout_ms = 30_000
    })
    .await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(200))
        .build()
        .unwrap();
    let sent = client
        .post(format!("{}/v1/chat/completions", r.base))
        .json(&user("hi"))
        .send()
        .await;
    assert!(
        sent.is_err(),
        "the fake never answers, so the client must time out"
    );
    let l = &log_lines_wait(&r.dir.log(), 1).await[0];
    assert_eq!(
        (l["outcome"].as_str(), l["target"].as_str()),
        (Some("client_disconnect"), Some("local"))
    );
    assert_eq!(r.cloud.count(), 0, "a cancelled request must not fail over");
}
