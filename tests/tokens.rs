//! The token estimate against llama3.2:1b's own counts (Ollama `prompt_eval_count`, raw mode),
//! captured live on samples never used to design the estimator. Regenerate only with fresh text.

use serde::Deserialize;
use slm_router::tokens::estimate;

#[derive(Deserialize)]
struct Sample {
    name: String,
    llama3_2_tokens: usize,
    text: String,
}

fn samples() -> Vec<Sample> {
    serde_json::from_str(include_str!("fixtures/token_samples.json")).unwrap()
}

#[test]
fn estimate_stays_within_guard_of_llama_counts() {
    let mut bad = Vec::new();
    for s in samples() {
        let ratio = estimate(&s.text) as f64 / s.llama3_2_tokens as f64;
        println!("{:16} est/true = {ratio:.2}", s.name);
        // Under 0.85 risks silent truncation; over 1.5 sends too much to the cloud.
        if !(0.85..=1.5).contains(&ratio) {
            bad.push(format!("{} {ratio:.2}", s.name));
        }
    }
    assert!(bad.is_empty(), "outside 0.85..=1.5: {bad:?}");
}

#[test]
fn samples_cover_dense_and_short_text() {
    let names: Vec<String> = samples().into_iter().map(|s| s.name).collect();
    for want in [
        "app_log",
        "json",
        "csv",
        "rust",
        "prose",
        "cjk",
        "short_math",
    ] {
        assert!(names.iter().any(|n| n == want), "missing sample {want}");
    }
}

#[test]
fn empty_text_is_zero_and_whitespace_is_free() {
    assert_eq!(estimate(""), 0);
    assert_eq!(estimate("   \n\t "), 0);
}
