//! `slm-router route` decides exactly as a served request does, without calling an upstream.

mod common;
use common::*;
use serde_json::{Value, json};
use slm_router::route;

const PROMPTS: [&str; 3] = [
    "What is the capital of France?",
    "Prove that there are infinitely many primes, step by step.",
    "Explain how CUDA warp divergence affects kernel throughput.",
];

fn jsonl(prompts: &[&str]) -> String {
    prompts
        .iter()
        .map(|p| json!({"prompt": p, "label": "ignored"}).to_string())
        .collect::<Vec<_>>()
        .join("\n\n") // blank lines between rows are skipped
}

fn lines(out: &str) -> Vec<Value> {
    out.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[tokio::test]
async fn route_matches_what_the_served_router_logs() {
    let r = rig(Mode::Ok, Mode::Ok, |_| {}).await;
    for p in PROMPTS {
        let (status, _) = post_chat(&r.base, &user(p), &[]).await;
        assert_eq!(status, 200);
    }
    let logged = log_lines(&r.dir.log());
    let cfg = config("http://x/v1", "http://y/v1", &r.dir.log());
    let routed = lines(&route::run(cfg, &jsonl(&PROMPTS)).unwrap());
    assert_eq!(routed.len(), PROMPTS.len());
    for (l, o) in logged.iter().zip(&routed) {
        for k in ["tier", "target", "reason", "classifier_score"] {
            assert_eq!(l[k], o[k], "{k}: log {l} vs route {o}");
        }
        assert!(o["route_us"].as_f64().is_some_and(|us| us >= 0.0), "{o}");
    }
    // Lines are numbered as in the file, blank lines included.
    let nums: Vec<u64> = routed.iter().map(|o| o["line"].as_u64().unwrap()).collect();
    assert_eq!(nums, [1, 3, 5]);
    assert_eq!(routed[0]["reason"], "default");
    assert_eq!(routed[1]["reason"], "complexity");
}

#[test]
fn a_malformed_row_names_its_line() {
    let cfg = config(
        "http://x/v1",
        "http://y/v1",
        std::path::Path::new("unused.jsonl"),
    );
    let err = route::run(cfg, "{\"prompt\": \"hi\"}\n{\"text\": \"no prompt\"}").unwrap_err();
    assert!(err.contains("line 2"), "{err}");
}

#[test]
fn the_routing_bench_fixture_is_balanced_and_routable() {
    let rows: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/routing_bench.json")).unwrap();
    assert_eq!(rows.len(), 120);
    for cat in [
        "trivia",
        "summarization",
        "coding",
        "debugging",
        "reasoning",
    ] {
        let of: Vec<&Value> = rows.iter().filter(|r| r["category"] == cat).collect();
        let local = of.iter().filter(|r| r["label"] == "local").count();
        assert_eq!((of.len(), local), (24, 12), "{cat}");
    }
    assert!(
        rows.iter()
            .all(|r| r["label"] == "local" || r["label"] == "cloud")
    );
    let text: Vec<String> = rows.iter().map(Value::to_string).collect();
    let cfg = config(
        "http://x/v1",
        "http://y/v1",
        std::path::Path::new("unused.jsonl"),
    );
    let out = route::run(cfg, &text.join("\n")).unwrap();
    assert_eq!(out.lines().count(), 120);
    assert!(!out.contains("\"error\""), "{out}");
}
