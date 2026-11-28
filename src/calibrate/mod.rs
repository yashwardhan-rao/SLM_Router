//! `slm-router calibrate`: suggest a Rule 5 `classifier_threshold` from your own labelled
//! prompts. The threshold optimises a target (best F1 by default, or a local-retention floor, or
//! a false-local ceiling) instead of keeping every local row local, so one noisy or mislabelled
//! prompt cannot drag it to an extreme. Pure: text in, report out.

use crate::routing::{Features, RoutingConfig, classifier_score, cued};
use serde::Deserialize;

mod pick;
pub use pick::{Metrics, Point, Target, candidates, eval, pick};

/// Added above the highest score kept local when nothing scores higher, as in `scripts/train.py`.
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
    pub target: Target,
    pub suggested: Metrics,
    /// The threshold in effect now, on the same rows.
    pub current: Metrics,
    /// The old rule: the most local rows kept, then the most cloud rows caught.
    pub strict: Metrics,
    /// Rows over `complex_prompt_tokens`: rule 5 sends those to the cloud without the classifier.
    pub excluded: usize,
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

pub fn threshold(
    rows: &[Labelled],
    cfg: &RoutingConfig,
    target: Target,
) -> Result<Outcome, String> {
    let on = RoutingConfig {
        classifier_threshold: 1.0,
        ..cfg.clone()
    };
    let points: Vec<Point> = rows
        .iter()
        .filter_map(|r| {
            score_on(&r.prompt, &on).map(|p| Point {
                p,
                cued: cued(&r.prompt, cfg),
                cloud: r.cloud,
            })
        })
        .collect();
    if points.iter().all(|x| x.cloud) {
        return Err("no local rows to calibrate on (a threshold needs at least one prompt labelled \"local\" within complex_prompt_tokens)".into());
    }
    let top = eval(&points, f32::INFINITY).retention();
    Ok(Outcome {
        target,
        suggested: pick(&points, target)?,
        current: eval(&points, cfg.classifier_threshold),
        strict: pick(&points, Target::MinLocalRecall(top))?,
        excluded: rows.len() - points.len(),
    })
}

fn pct(v: Option<f64>) -> String {
    v.map_or("n/a".into(), |v| format!("{:.1}%", v * 100.0))
}

fn row(name: &str, m: &Metrics) -> String {
    format!(
        "{name:<24}{:>9.3}{:>12}{:>13}{:>10}",
        m.threshold,
        pct(Some(m.retention())),
        pct(m.false_local()),
        pct(Some(m.accuracy()))
    )
}

/// What `calibrate` prints: the comparison table, then the suggestion.
pub fn render(o: &Outcome) -> String {
    let name = match o.target {
        Target::F1 => "suggested (best F1)".to_string(),
        Target::MinLocalRecall(r) => format!("suggested (kept>={:.0}%)", r * 100.0),
        Target::MaxFalseLocal(f) => format!("suggested (false<={:.0}%)", f * 100.0),
    };
    let s = &o.suggested;
    let mut out = vec![
        format!(
            "{:<24}{:>9}{:>12}{:>13}{:>10}",
            "", "threshold", "local kept", "false local", "accuracy"
        ),
        row(&name, s),
        row("current", &o.current),
        row("strict (keep all)", &o.strict),
        String::new(),
        format!("classifier_threshold = {:.3}", s.threshold),
        format!(
            "rows: {} local, {} cloud ({} over complex_prompt_tokens skipped)",
            s.local, s.cloud, o.excluded
        ),
    ];
    if s.threshold > 1.0 {
        out.push("note: above 1.0, this threshold disables the classifier".into());
    }
    if s.local < FEW_LOCAL {
        out.push(format!(
            "warning: only {} local rows; label at least {FEW_LOCAL} for a stable threshold",
            s.local
        ));
    }
    out.join(
        "
",
    )
}

/// clap parser for the target flags: a fraction from 0 to 1.
pub fn fraction(s: &str) -> Result<f64, String> {
    s.parse::<f64>()
        .ok()
        .filter(|v| (0.0..=1.0).contains(v))
        .ok_or_else(|| format!("{s:?} must be a number from 0 to 1, e.g. 0.95"))
}
