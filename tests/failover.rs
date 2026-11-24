mod common;
use common::*;
use serde_json::json;

#[tokio::test]
async fn refused_local_connection_fails_over_to_cloud() {
    let r = rig(Mode::Ok, Mode::Ok, |c| c.local.base_url = dead_url()).await;
    let (status, body) = post_chat(&r.base, &user("hi"), &[]).await;
    assert_eq!((status, body["model"].as_str()), (200, Some("cloud")));
    assert_eq!(r.cloud.last().body["model"], "cloud-test");
}

#[tokio::test]
async fn local_5xx_fails_over_to_cloud() {
    let r = rig(Mode::Status(503, "{}"), Mode::Ok, |_| {}).await;
    let (status, body) = post_chat(&r.base, &user("hi"), &[]).await;
    assert_eq!((status, body["model"].as_str()), (200, Some("cloud")));
    assert_eq!(r.local.count(), 1);
}

#[tokio::test]
async fn local_first_byte_timeout_fails_over_to_cloud() {
    let r = rig(Mode::Hang, Mode::Ok, |_| {}).await;
    let (status, body) = post_chat(&r.base, &user("hi"), &[]).await;
    assert_eq!((status, body["model"].as_str()), (200, Some("cloud")));
}

#[tokio::test]
async fn local_4xx_is_the_clients_error_and_does_not_fail_over() {
    let r = rig(Mode::Status(400, r#"{"error":"bad"}"#), Mode::Ok, |_| {}).await;
    let (status, body) = post_chat(&r.base, &user("hi"), &[]).await;
    assert_eq!((status, body), (400, json!({"error": "bad"})));
    assert_eq!(r.cloud.count(), 0);
}

#[tokio::test]
async fn local_only_policy_never_leaks_to_cloud_when_local_is_down() {
    let r = rig(Mode::Ok, Mode::Ok, |c| c.local.base_url = dead_url()).await;
    let (status, body) =
        post_chat(&r.base, &user("hi"), &[("x-router-policy", "local-only")]).await;
    assert_eq!(status, 503);
    assert!(body["error"]["message"].is_string());
    assert_eq!(r.cloud.count(), 0);
}

#[tokio::test]
async fn forced_local_model_does_not_fail_over() {
    let r = rig(Mode::Ok, Mode::Ok, |c| c.local.base_url = dead_url()).await;
    let mut req = user("hi");
    req["model"] = json!("slm-router/local");
    let (status, _) = post_chat(&r.base, &req, &[]).await;
    assert_eq!(status, 503);
    assert_eq!(r.cloud.count(), 0);
}

#[tokio::test]
async fn cloud_errors_pass_through_with_no_retry_and_no_local_fallback() {
    let r = rig(
        Mode::Ok,
        Mode::Status(429, r#"{"error":"slow down"}"#),
        |_| {},
    )
    .await;
    let mut req = user("hi");
    req["model"] = json!("slm-router/cloud");
    let (status, body) = post_chat(&r.base, &req, &[]).await;
    assert_eq!((status, body), (429, json!({"error": "slow down"})));
    assert_eq!((r.cloud.count(), r.local.count()), (1, 0));
}
