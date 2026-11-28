//! Failover and circuit breakers on a three-tier `[[tier]]` ladder: up the ladder only.

mod common;
use common::*;
use serde_json::{Value, json};

fn with_tools() -> Value {
    let mut req = user("hi");
    req["tools"] = json!([{"type": "function", "function": {"name": "f", "parameters": {}}}]);
    req
}

fn summary(l: &Value) -> (&str, &str, bool) {
    let s = |k: &str| l[k].as_str().unwrap_or_default();
    (s("tier"), s("reason"), l["failed_over"] == true)
}

#[tokio::test]
async fn failover_walks_up_two_tiers_and_logs_the_first_hops_cause() {
    let dead = [Mode::Ok, Mode::Status(503, "{}"), Mode::Ok];
    let r = rig3(dead, |c| c.tier[0].base_url = dead_url()).await;
    let (status, body) = post_chat(&r.base, &user("hi"), &[]).await;
    assert_eq!((status, body["model"].as_str()), (200, Some("top")));
    assert_eq!(r.t[1].count(), 1, "mid was tried on the way up");
    let l = &log_lines(&r.dir.log())[0];
    assert_eq!(summary(l), ("top", "local_unavailable", true));
}

#[tokio::test]
async fn failover_skips_a_tier_without_the_capabilities_the_request_needs() {
    let r = rig3([Mode::Ok, Mode::Ok, Mode::Ok], |c| {
        c.tier[0].base_url = dead_url();
        c.tier[0].supports_tools = true;
        c.tier[1].supports_tools = false;
    })
    .await;
    let (status, body) = post_chat(&r.base, &with_tools(), &[]).await;
    assert_eq!((status, body["model"].as_str()), (200, Some("top")));
    assert_eq!(r.t[1].count(), 0);
}

#[tokio::test]
async fn local_only_fails_over_between_local_tiers_and_never_leaves_them() {
    let r = rig3([Mode::Ok, Mode::Ok, Mode::Ok], |c| {
        c.tier[0].base_url = dead_url();
        c.tier[1].local = true;
    })
    .await;
    let lo = [("x-router-policy", "local-only")];
    let (status, body) = post_chat(&r.base, &user("hi"), &lo).await;
    assert_eq!((status, body["model"].as_str()), (200, Some("mid")));

    let r = rig3([Mode::Ok, Mode::Ok, Mode::Ok], |c| {
        c.tier[0].base_url = dead_url();
        c.tier[1].base_url = dead_url();
        c.tier[1].local = true;
    })
    .await;
    let (status, body) = post_chat(&r.base, &user("hi"), &lo).await;
    assert_eq!(
        (status, body["error"]["code"].as_str()),
        (503, Some("local_unavailable"))
    );
    assert_eq!(r.t[2].count(), 0);
}

#[tokio::test]
async fn a_remote_tier_with_an_open_circuit_is_skipped_as_circuit_open() {
    let r = rig3([Mode::Ok, Mode::Status(503, "{}"), Mode::Ok], |_| {}).await;
    for _ in 0..3 {
        let (status, body) = post_chat(&r.base, &with_tools(), &[]).await;
        assert_eq!((status, body["model"].as_str()), (200, Some("top")));
    }
    assert_eq!(r.t[1].count(), 2, "the third request must not reach mid");
    let log = log_lines(&r.dir.log());
    assert_eq!(summary(&log[0]), ("top", "tier_unavailable", true));
    assert_eq!(summary(&log[2]), ("top", "circuit_open", true));
}

#[tokio::test]
async fn an_open_circuit_on_the_last_tier_is_a_503_not_a_bypass() {
    let r = rig3([Mode::Ok, Mode::Ok, Mode::Status(503, "{}")], |_| {}).await;
    let mut req = user("hi");
    req["model"] = json!("slm-router/top");
    for _ in 0..2 {
        let (status, _) = post_chat(&r.base, &req, &[]).await;
        assert_eq!(status, 503, "a remote tier's own 5xx is relayed");
    }
    let (status, body) = post_chat(&r.base, &req, &[]).await;
    assert_eq!(
        (status, body["error"]["code"].as_str()),
        (503, Some("circuit_open"))
    );
    assert_eq!(r.t[2].count(), 2);
}

#[tokio::test]
async fn a_disabled_breaker_never_skips_its_tier() {
    let r = rig3([Mode::Ok, Mode::Status(503, "{}"), Mode::Ok], |c| {
        c.tier[1].breaker_failures = 0
    })
    .await;
    for _ in 0..4 {
        post_chat(&r.base, &with_tools(), &[]).await;
    }
    assert_eq!(r.t[1].count(), 4);
}
