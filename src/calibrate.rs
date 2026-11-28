//! `slm-router calibrate`: suggest a Rule 5 `classifier_threshold` from your own labelled
//! prompts, by the rule `scripts/train.py` uses: the highest score any `local` prompt gets, plus
//! a margin, so no labelled-local prompt is sent to the cloud. Pure: text in, report out.

use crate::routing::{Features, RoutingConfig, classifier_score, cued};
use serde::Deserialize;

/// Added to the highest local score, as in `scripts/train.py`.
pub const MARGIN: f32 = 0.02;
/// Below this many scored local rows the suggestion is printed with a warning.
const FEW_LOCAL: usize = 20;

#[derive(Debug, Deserialize)]
struct Row {
    prompt: String,
    label: String,
}

/// One labelled prompt: `cloud` is true when the local model's answer was not good enough.
#[derive(Debug, Clone)]
pub struct Labelled {
    pub prompt: String,
    pub cloud: bool,
}

/// Parse JSONL rows `{"prompt": ..., "label": "local" | "cloud"}`; other fields are ignored.
/// Blank lines are skipped; errors name the 1-based line, because a skipped row would bias the result.
pub fn parse(text: &str) -> Result<Vec<Labelled>, String> {
    let mut rows = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let r: Row =
            serde_json::from_str(line).map_err(|e| format!("labels line {}: {e}", i + 1))?;
        let cloud = match r.label.as_str() {
            "local" => false,
            "cloud" => true,
            other => {
                return Err(format!(
                    "labels line {}: label {other:?} must be \"local\" or \"cloud\"",
                    i + 1
                ));
            }
        };
        rows.push(Labelled {
            prompt: r.prompt,
            cloud,
        });
    }
    Ok(rows)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// Rounded up to 3 decimals, so rounding never drops it below max local + margin.
    pub threshold: f32,
    pub local: usize,
    pub cloud: usize,
    /// Rows over `complex_prompt_tokens`: rule 5 sends those to the cloud without the classifier.
    pub excluded: usize,
    /// Cloud rows rule 5 sends to the cloud at `threshold` (cues or classifier).
    pub caught: usize,
}

/// The classifier's score for a one-message prompt, exactly as the router logs it, but computed
/// even when the configured threshold disables the classifier. None when rule 5 skips the prompt.
pub fn score(prompt: &str, cfg: &RoutingConfig) -> Option<f32> {
    let on = RoutingConfig {
        classifier_threshold: 1.0,
        ..cfg.clone()
    };
    score_on(prompt, &on)
}

fn score_on(prompt: &str, on: &RoutingConfig) -> Option<f32> {
    let f = Features {
        // A single user message: its estimate plus the 4 template tokens, as the router counts it.
        est_prompt_tokens: crate::tokens::estimate(prompt) + 4,
        last_user_text: prompt.to_string(),
        ..Features::default()
    };
    classifier_score(&f, on)
}

pub fn threshold(rows: &[Labelled], cfg: &RoutingConfig) -> Result<Outcome, String> {
    let on = RoutingConfig {
        classifier_threshold: 1.0,
        ..cfg.clone()
    };
    let scored: Vec<(&Labelled, f32)> = rows
        .iter()
        .filter_map(|r| score_on(&r.prompt, &on).map(|p| (r, p)))
        .collect();
    let max_local = scored
        .iter()
        .filter(|(r, _)| !r.cloud)
        .map(|&(_, p)| p)
        .fold(None, |m: Option<f32>, p| Some(m.map_or(p, |m| m.max(p))))
        .ok_or("no local rows to calibrate on (a threshold needs at least one prompt labelled \"local\" within complex_prompt_tokens)")?;
    let t = ((max_local + MARGIN) * 1000.0).ceil() / 1000.0;
    let caught = scored
        .iter()
        .filter(|(r, p)| r.cloud && (*p >= t || cued(&r.prompt, cfg)))
        .count();
    let local = scored.iter().filter(|(r, _)| !r.cloud).count();
    Ok(Outcome {
        threshold: t,
        local,
        cloud: scored.len() - local,
        excluded: rows.len() - scored.len(),
        caught,
    })
}

/// What `calibrate` prints; `current` is the threshold in effect now.
pub fn render(o: &Outcome, current: f32) -> String {
    let mut out = vec![
        format!("classifier_threshold = {:.3}", o.threshold),
        format!(
            "rows: {} local, {} cloud ({} over complex_prompt_tokens skipped)",
            o.local, o.cloud, o.excluded
        ),
        match o.cloud {
            0 => "cloud rows caught by rule 5: n/a (no cloud rows)".to_string(),
            n => format!(
                "cloud rows caught by rule 5: {} / {n} ({:.1}%)",
                o.caught,
                o.caught as f64 * 100.0 / n as f64
            ),
        },
        format!("current classifier_threshold: {current:.3}"),
    ];
    if o.threshold > 1.0 {
        out.push("note: above 1.0, this threshold disables the classifier (a local row scored too high to separate)".into());
    }
    if o.local < FEW_LOCAL {
        out.push(format!(
            "warning: only {} local rows; label at least {FEW_LOCAL} for a stable threshold",
            o.local
        ));
    }
    out.join("\n")
}
