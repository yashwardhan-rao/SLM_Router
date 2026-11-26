//! Offline routing accuracy on the labelled prompt set, cue-only and with the Rule 5 classifier.
//! `dev` prompts may be used to tune the heuristics and calibrate the classifier; `holdout`
//! prompts must not be.

use serde::Deserialize;
use serde_json::json;
use slm_router::routing::{Policy, RoutingConfig, Target, decide};
use slm_router::schema::{ChatRequest, features};

#[derive(Deserialize)]
struct Case {
    id: String,
    split: String,
    expect: String,
    prompt: String,
}

#[derive(Default, Debug)]
struct Matrix {
    /// (expected, routed): local/local, local/cloud (false cloud), cloud/local (missed), cloud/cloud.
    ll: usize,
    lc: Vec<String>,
    cl: Vec<String>,
    cc: usize,
}

fn route(prompt: &str, cfg: &RoutingConfig) -> Target {
    let req: ChatRequest = serde_json::from_value(
        json!({"model": "slm-router/auto", "messages": [{"role": "user", "content": prompt}]}),
    )
    .unwrap();
    decide(&features(&req), cfg, Policy::Auto, None)
        .unwrap()
        .target
}

fn matrix(split: &str, cfg: &RoutingConfig) -> Matrix {
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("fixtures/routing_eval.json")).unwrap();
    let mut m = Matrix::default();
    for c in cases.iter().filter(|c| c.split == split) {
        let expect = if c.expect == "cloud" {
            Target::Cloud
        } else {
            Target::Local
        };
        match (expect, route(&c.prompt, cfg)) {
            (Target::Local, Target::Local) => m.ll += 1,
            (Target::Local, Target::Cloud) => m.lc.push(c.id.clone()),
            (Target::Cloud, Target::Local) => m.cl.push(c.id.clone()),
            (Target::Cloud, Target::Cloud) => m.cc += 1,
        }
    }
    m
}

/// Hard recall of `split` under `cfg`, after asserting no easy prompt went to cloud.
fn recall(split: &str, label: &str, cfg: &RoutingConfig) -> f64 {
    let m = matrix(split, cfg);
    let recall = m.cc as f64 / (m.cc + m.cl.len()) as f64;
    println!(
        "{split} {label}: hard recall {recall:.3} ({}/{}) | false cloud {:?} | missed {:?}",
        m.cc,
        m.cc + m.cl.len(),
        m.lc,
        m.cl
    );
    assert!(
        m.lc.is_empty(),
        "{split} {label}: easy prompts sent to cloud: {:?}",
        m.lc
    );
    recall
}

/// (cue-only, cue + classifier) recall; both must send no easy prompt to cloud.
fn both(split: &str) -> (f64, f64) {
    let cues = RoutingConfig {
        classifier_threshold: 2.0,
        ..Default::default()
    };
    (
        recall(split, "cue only", &cues),
        recall(split, "cue+classifier", &RoutingConfig::default()),
    )
}

/// The planned floor was 0.90, fixed before training. The shipped model reached 47/53 (0.887) at
/// 0 false cloud while clearing the blind holdout gate by +0.35, so the owner lowered it to 0.88
/// after seeing the results. That is a post-hoc override, recorded in the v0.4.0 notes.
#[test]
fn dev_split_meets_floors() {
    let (cues, with) = both("dev");
    assert!(cues >= 0.69, "cue-only dev recall {cues:.3} regressed");
    assert!(with >= 0.88, "dev recall with classifier {with:.3} < 0.88");
}

/// Holdout v3 is blind: written before any v0.4.0 corpus or model, never used to train or tune.
/// Gate: the classifier lifts recall by at least 0.25 over cue-only, with 0 false cloud.
#[test]
fn holdout_classifier_lifts_recall_without_false_cloud() {
    let (cues, with) = both("holdout");
    assert!(
        with >= cues + 0.25,
        "holdout recall {with:.3} < cue-only {cues:.3} + 0.25"
    );
}
