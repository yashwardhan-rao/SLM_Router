//! Routing, pins, logging and savings over a three-tier `[[tier]]` ladder.

mod common;
use common::*;
use serde_json::{Value, json};

const HARD: &str = "prove that the sum of two even numbers is even";

fn with_tools(text: &str) -> Value {
    let mut req = user(text);
    req["tools"] = json!([{"type": "function", "function": {"name": "f", "parameters": {}}}]);
    req
}

fn field<'a>(l: &'a Value, k: &str) -> &'a str {
    l[k].as_str().unwrap_or_default()
}

#[tokio::test]
async fn each_request_goes_to_the_cheapest_tier_that_can_serve_it() {
    let r = rig3([Mode::Ok, Mode::Ok, Mode::Ok], |_| {}).await;
    let cases = [
        (user("hi"), "small"),
        (with_tools("hi"), "mid"),
        (user(HARD), "top"),
    ];
    for (req, want) in &cases {
        let (status, body) = post_chat(&r.base, req, &[]).await;
        assert_eq!((status, body["model"].as_str()), (200, Some(*want)));
    }
    assert_eq!(r.t[2].last().auth.as_deref(), Some("Bearer sk-top"));
    assert_eq!(r.t[1].last().auth.as_deref(), Some("Bearer sk-mid"));
    assert_eq!(r.t[0].last().auth, None);
    let log = log_lines(&r.dir.log());
    let got: Vec<_> = log
        .iter()
        .map(|l| (field(l, "tier"), field(l, "target"), field(l, "reason")))
        .collect();
    assert_eq!(
        got,
        [
            ("small", "local", "default"),
            ("mid", "cloud", "capability"),
            ("top", "cloud", "complexity"),
        ]
    );
}

#[tokio::test]
async fn saved_usd_is_measured_against_the_top_tier() {
    let r = rig3([Mode::Ok, Mode::Ok, Mode::Ok], |_| {}).await;
    for req in [user("hi"), with_tools("hi"), user(HARD)] {
        post_chat(&r.base, &req, &[]).await;
    }
    // The fakes report 10 prompt + 5 completion tokens; top costs (10*3 + 5*15) / 1e6.
    let saved: Vec<f64> = log_lines(&r.dir.log())
        .iter()
        .map(|l| l["saved_usd"].as_f64().unwrap())
        .collect();
    let close = |a: f64, b: f64| (a - b).abs() < 1e-12;
    assert!(close(saved[0], 0.000105), "{saved:?}");
    assert!(close(saved[1], 0.000105 - 0.00002), "{saved:?}");
    assert!(close(saved[2], 0.0), "{saved:?}");
}

#[tokio::test]
async fn a_pin_escalates_with_the_session_and_never_comes_back_down() {
    let r = rig3([Mode::Ok, Mode::Ok, Mode::Ok], |_| {}).await;
    let s = [("x-session-id", "s1")];
    let mut tiers = vec![];
    for req in [user("hi"), with_tools("hi"), user("hi again")] {
        let (_, body) = post_chat(&r.base, &req, &s).await;
        tiers.push(body["model"].as_str().unwrap().to_string());
    }
    assert_eq!(tiers, ["small", "mid", "mid"]);
    let reasons: Vec<_> = log_lines(&r.dir.log())
        .iter()
        .map(|l| field(l, "reason").to_string())
        .collect();
    assert_eq!(reasons, ["default", "capability", "pinned"]);
}

#[tokio::test]
async fn a_tier_can_be_forced_by_name() {
    let r = rig3([Mode::Ok, Mode::Ok, Mode::Ok], |_| {}).await;
    let mut req = user(HARD);
    req["model"] = json!("slm-router/mid");
    let (status, body) = post_chat(&r.base, &req, &[]).await;
    assert_eq!((status, body["model"].as_str()), (200, Some("mid")));
    assert_eq!(r.t[1].last().body["model"], "mid-m");
    assert_eq!(field(&log_lines(&r.dir.log())[0], "reason"), "forced");
}

#[tokio::test]
async fn models_lists_tiers_and_only_aliases_that_resolve() {
    let up = fake(Mode::Ok, "top").await;
    let dir = TempDir::new();
    let toml = format!(
        "[[tier]]\nname = \"only\"\nbase_url = \"{}\"\nmodel = \"m\"\n[log]\npath = {:?}\n",
        up.url,
        dir.log().to_str().unwrap()
    );
    let base = serve(slm_router::config::Config::from_toml(&toml).unwrap()).await;
    let v: Value = reqwest::get(format!("{base}/v1/models"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<&str> = v["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        ["slm-router/auto", "slm-router/cloud", "slm-router/only"]
    );
    let mut req = user("hi");
    req["model"] = json!("slm-router/local");
    let (status, body) = post_chat(&base, &req, &[]).await;
    assert_eq!(
        (status, body["error"]["code"].as_str()),
        (400, Some("model_not_found"))
    );
    assert_eq!(up.count(), 0);
}

#[tokio::test]
async fn a_request_routed_under_a_policy_does_not_pin_the_session() {
    let r = rig3([Mode::Ok, Mode::Ok, Mode::Ok], |_| {}).await;
    let sid = ("x-session-id", "s2");
    let (_, body) = post_chat(
        &r.base,
        &user("hi"),
        &[sid, ("x-router-policy", "cloud-only")],
    )
    .await;
    assert_eq!(body["model"], "mid");
    let (_, body) = post_chat(&r.base, &user("hi"), &[sid]).await;
    assert_eq!(body["model"], "small");
    assert_eq!(field(&log_lines(&r.dir.log())[1], "reason"), "default");
}
