mod common;
use common::*;
use serde_json::{Value, json};

fn tools_req(text: &str) -> Value {
    let mut r = user(text);
    r["tools"] = json!([{"type": "function", "function": {"name": "t"}}]);
    r
}

#[tokio::test]
async fn session_header_pins_a_conversation_to_cloud_after_escalation() {
    let r = rig(Mode::Ok, Mode::Ok, |_| {}).await;
    let h = [("x-session-id", "s1")];
    post_chat(&r.base, &tools_req("start"), &h).await;
    let (_, body) = post_chat(&r.base, &user("just chat"), &h).await;
    assert_eq!(body["model"], "cloud");
    assert_eq!(r.local.count(), 0);
}

#[tokio::test]
async fn other_sessions_are_not_affected_by_a_pin() {
    let r = rig(Mode::Ok, Mode::Ok, |_| {}).await;
    post_chat(&r.base, &tools_req("start"), &[("x-session-id", "s1")]).await;
    let (_, body) = post_chat(&r.base, &user("hi"), &[("x-session-id", "s2")]).await;
    assert_eq!(body["model"], "local");
}

#[tokio::test]
async fn without_a_header_the_session_is_fingerprinted_from_system_and_first_user_message() {
    let r = rig(Mode::Ok, Mode::Ok, |_| {}).await;
    let sys = json!({"role": "system", "content": "be brief"});
    let first = json!({"role": "user", "content": "plan my trip"});
    let mut turn1 = tools_req("x");
    turn1["messages"] = json!([sys, first]);
    post_chat(&r.base, &turn1, &[]).await;
    let mut turn2 = user("x");
    turn2["messages"] = json!([sys, first, {"role": "assistant", "content": "ok"}, {"role": "user", "content": "thanks"}]);
    let (_, body) = post_chat(&r.base, &turn2, &[]).await;
    assert_eq!(body["model"], "cloud");
}

#[tokio::test]
async fn failover_does_not_pin_the_session_to_cloud() {
    let r = rig(Mode::Status(503, "{}"), Mode::Ok, |_| {}).await;
    let h = [("x-session-id", "s1")];
    post_chat(&r.base, &user("hi"), &h).await;
    post_chat(&r.base, &user("again"), &h).await;
    assert_eq!(r.local.count(), 2, "second request must try local again");
    assert_ne!(log_lines(&r.dir.log())[1]["reason"], "pinned");
}

#[tokio::test]
async fn forced_requests_do_not_create_pins() {
    let r = rig(Mode::Ok, Mode::Ok, |_| {}).await;
    let h = [("x-session-id", "s1")];
    let mut forced = user("hi");
    forced["model"] = json!("slm-router/cloud");
    post_chat(&r.base, &forced, &h).await;
    let (_, body) = post_chat(&r.base, &user("hi"), &h).await;
    assert_eq!(body["model"], "local");
}
