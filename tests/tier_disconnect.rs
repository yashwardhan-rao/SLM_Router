//! A client that hangs up mid-request is logged against the tier that was being tried.

mod common;
use common::*;
use serde_json::{Value, json};
use std::time::Duration;

/// Sends `body` and gives up after 300 ms, as an impatient client would.
async fn hang_up(base: &str, body: &Value) {
    let c = reqwest::Client::builder()
        .timeout(Duration::from_millis(300))
        .build()
        .unwrap();
    let r = c
        .post(format!("{base}/v1/chat/completions"))
        .json(body)
        .send();
    assert!(
        r.await.is_err(),
        "the upstream hangs, so the client must time out"
    );
}

fn s<'a>(l: &'a Value, k: &str) -> &'a str {
    l[k].as_str().unwrap_or_default()
}

#[tokio::test]
async fn an_abandoned_cloud_request_is_logged_as_cloud() {
    let r = rig(Mode::Ok, Mode::Hang, |_| {}).await;
    let mut req = user("hi");
    req["model"] = json!("slm-router/cloud");
    hang_up(&r.base, &req).await;
    let l = &log_lines_wait(&r.dir.log(), 1).await[0];
    let got = (s(l, "target"), s(l, "tier"), s(l, "outcome"));
    assert_eq!(got, ("cloud", "cloud", "client_disconnect"));
}

#[tokio::test]
async fn a_hang_up_after_failover_is_logged_against_the_tier_being_tried() {
    let modes = [Mode::Ok, Mode::Status(503, "{}"), Mode::Hang];
    let r = rig3(modes, |c| c.tier[0].base_url = dead_url()).await;
    hang_up(&r.base, &user("hi")).await;
    let l = &log_lines_wait(&r.dir.log(), 1).await[0];
    let got = (s(l, "tier"), s(l, "reason"), l["failed_over"] == true);
    assert_eq!(got, ("top", "local_unavailable", true));
    assert_eq!(s(l, "outcome"), "client_disconnect");
}
