//! `slm-router route`: the routing decision for each prompt, offline. No upstream is called.
//! It runs the same `features()`, `decide()` and `classifier_score()` as a served request, so
//! benchmarks measure the shipped router rather than a reimplementation.

use crate::config::Config;
use crate::routing::{classifier_score, decide};
use crate::schema::{ChatRequest, features};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Instant;

/// Each prompt is routed this many times and the median time reported, to steady the clock.
const RUNS: usize = 11;

#[derive(Deserialize)]
struct Row {
    prompt: String,
}

/// One JSON line per prompt: tier, target, reason, classifier score and the median routing time
/// in microseconds. Input is JSONL with a `prompt` field; other fields are ignored.
pub fn run(cfg: Config, text: &str) -> Result<String, String> {
    let ladder = cfg.ladder();
    let mut routing = cfg.routing.clone();
    routing.tiers = ladder.iter().map(|t| t.spec()).collect();
    let mut out = Vec::new();
    let mut warm = false;
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: Row =
            serde_json::from_str(line).map_err(|e| format!("prompts line {}: {e}", i + 1))?;
        let req: ChatRequest = serde_json::from_value(json!({"model": "slm-router/auto",
            "messages": [{"role": "user", "content": row.prompt}]}))
        .expect("a one-message chat request always parses");
        // What a served request does before it calls an upstream: features, decision, logged score.
        let route = || {
            let f = features(&req);
            (
                decide(&f, &routing, cfg.server.policy, None),
                classifier_score(&f, &routing),
            )
        };
        if !warm {
            let _ = route(); // the first call decodes the embedded classifier
            warm = true;
        }
        let mut times = Vec::with_capacity(RUNS);
        let mut result = None;
        for _ in 0..RUNS {
            let t = Instant::now();
            let r = std::hint::black_box(route());
            times.push(t.elapsed().as_secs_f64() * 1e6);
            result = Some(r);
        }
        times.sort_by(f64::total_cmp);
        let (decision, score) = result.expect("RUNS > 0");
        let score = score.map(|p| (f64::from(p) * 1000.0).round() / 1000.0);
        let mut v = match decision {
            Ok(d) => json!({"tier": ladder[d.tier].name, "target": d.target, "reason": d.reason}),
            Err(e) => json!({"error": format!("{e:?}")}),
        };
        v["line"] = json!(i + 1);
        v["classifier_score"] = json!(score);
        v["route_us"] = json!((times[RUNS / 2] * 100.0).round() / 100.0);
        out.push(Value::to_string(&v));
    }
    Ok(out.join("\n"))
}
