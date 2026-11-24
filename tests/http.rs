mod common;
use common::*;
use serde_json::{Value, json};

async fn default_rig() -> Rig {
    rig(Mode::Ok, Mode::Ok, |_| {}).await
}

#[tokio::test]
async fn models_endpoint_lists_the_three_router_ids() {
    let r = default_rig().await;
    let v: Value = reqwest::get(format!("{}/v1/models", r.base))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<_> = v["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        ["slm-router/auto", "slm-router/local", "slm-router/cloud"]
    );
}

#[tokio::test]
async fn simple_request_goes_local_with_model_rewritten_and_response_relayed() {
    let r = default_rig().await;
    let (status, body) = post_chat(&r.base, &user("hi"), &[]).await;
    assert_eq!((status, body["model"].as_str()), (200, Some("local")));
    assert_eq!(r.local.last().body["model"], "llama-test");
    assert_eq!(r.cloud.count(), 0);
}

#[tokio::test]
async fn tools_request_goes_to_cloud_with_server_side_key_only() {
    let r = default_rig().await;
    let mut req = user("weather?");
    req["tools"] = json!([{"type": "function", "function": {"name": "w"}}]);
    let (status, _) = post_chat(&r.base, &req, &[("authorization", "Bearer client-secret")]).await;
    assert_eq!(status, 200);
    assert_eq!(r.cloud.last().auth.as_deref(), Some("Bearer sk-cloud"));
    assert_eq!(r.cloud.last().body["model"], "cloud-test");
    assert_eq!(r.local.count(), 0);
}

#[tokio::test]
async fn image_content_parts_route_to_cloud() {
    let r = default_rig().await;
    let part = json!([{"type": "image_url", "image_url": {"url": "http://x/y.png"}}]);
    let mut req = user("see");
    req["messages"][0]["content"] = part;
    post_chat(&r.base, &req, &[]).await;
    assert_eq!((r.cloud.count(), r.local.count()), (1, 0));
}

#[tokio::test]
async fn unknown_fields_pass_through_untouched() {
    let r = default_rig().await;
    let mut req = user("hi");
    req["seed"] = json!(7);
    req["x_custom"] = json!({"a": [1, 2]});
    req["messages"][0]["name"] = json!("bob");
    post_chat(&r.base, &req, &[]).await;
    let seen = r.local.last().body;
    assert_eq!(seen["seed"], 7);
    assert_eq!(seen["x_custom"], json!({"a": [1, 2]}));
    assert_eq!(seen["messages"][0]["name"], "bob");
}

#[tokio::test]
async fn forced_model_and_policy_header_conflict_is_a_400_openai_error() {
    let r = default_rig().await;
    let mut req = user("hi");
    req["model"] = json!("slm-router/cloud");
    let (status, body) = post_chat(&r.base, &req, &[("x-router-policy", "local-only")]).await;
    assert_eq!(status, 400);
    assert!(body["error"]["message"].is_string() && body["error"]["type"].is_string());
    assert_eq!(r.cloud.count() + r.local.count(), 0);
}

#[tokio::test]
async fn malformed_body_is_a_400_openai_error() {
    let r = default_rig().await;
    let (s, body) = post_chat(&r.base, &json!({"nope": 1}), &[]).await;
    assert_eq!(s, 400);
    assert!(body["error"]["message"].is_string());
}

#[tokio::test]
async fn router_bearer_token_is_enforced_when_configured() {
    let r = rig(Mode::Ok, Mode::Ok, |c| c.server.api_key = Some("k".into())).await;
    let models = format!("{}/v1/models", r.base);
    assert_eq!(reqwest::get(&models).await.unwrap().status(), 401);
    let (s, _) = post_chat(&r.base, &user("hi"), &[("authorization", "Bearer wrong")]).await;
    assert_eq!(s, 401);
    let (s, _) = post_chat(&r.base, &user("hi"), &[("authorization", "Bearer k")]).await;
    assert_eq!(s, 200);
}
