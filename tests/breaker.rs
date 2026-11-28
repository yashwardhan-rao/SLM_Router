mod common;
use common::*;
use serde_json::json;
use std::time::Duration;

/// Router whose local always answers 503; the breaker opens after 2 failures for 200ms.
async fn dead_local() -> Rig {
    rig(Mode::Status(503, "{}"), Mode::Ok, |c| {
        c.local.as_mut().unwrap().breaker_cooldown_ms = 200;
    })
    .await
}

#[tokio::test]
async fn third_request_skips_a_dead_local_and_is_logged_as_a_failover() {
    let r = dead_local().await;
    for _ in 0..3 {
        let (status, body) = post_chat(&r.base, &user("hi"), &[]).await;
        assert_eq!((status, body["model"].as_str()), (200, Some("cloud")));
    }
    assert_eq!(r.local.count(), 2, "third request must not reach local");
    let l = &log_lines(&r.dir.log())[2];
    assert_eq!(
        (
            l["reason"].as_str(),
            l["failed_over"].as_bool(),
            l["status"].as_u64()
        ),
        (Some("circuit_open"), Some(true), Some(200))
    );
    let log = std::fs::read(r.dir.log()).unwrap();
    let report = slm_router::stats::report(&log[..], u64::MAX, None);
    assert!(report.contains("failovers: 3"), "{report}");
}

#[tokio::test]
async fn forced_local_with_an_open_circuit_is_an_immediate_503() {
    let r = dead_local().await;
    post_chat(&r.base, &user("a"), &[]).await;
    post_chat(&r.base, &user("b"), &[]).await;
    let mut req = user("c");
    req["model"] = json!("slm-router/local");
    let (status, body) = post_chat(&r.base, &req, &[]).await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["error"]["code"], "local_unavailable");
    assert_eq!(r.local.count(), 2, "local must not be contacted");
    let (status, _) = post_chat(&r.base, &user("d"), &[("x-router-policy", "local-only")]).await;
    assert_eq!((status, r.local.count()), (503, 2));
}

#[tokio::test]
async fn after_the_cooldown_one_probe_reaches_local() {
    let r = dead_local().await;
    post_chat(&r.base, &user("a"), &[]).await;
    post_chat(&r.base, &user("b"), &[]).await;
    tokio::time::sleep(Duration::from_millis(250)).await;
    post_chat(&r.base, &user("probe"), &[]).await;
    assert_eq!(r.local.count(), 3);
    post_chat(&r.base, &user("after a failed probe"), &[]).await;
    assert_eq!(r.local.count(), 3, "the failed probe reopened the breaker");
}

#[tokio::test]
async fn local_4xx_does_not_count_toward_opening() {
    let r = rig(Mode::Status(400, r#"{"error":"bad"}"#), Mode::Ok, |_| {}).await;
    for _ in 0..3 {
        post_chat(&r.base, &user("hi"), &[]).await;
    }
    assert_eq!((r.local.count(), r.cloud.count()), (3, 0));
}
