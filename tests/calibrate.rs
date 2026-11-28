mod common;

use common::*;
use serde_json::json;
use slm_router::calibrate::{self, Labelled, Outcome, parse, render, threshold};
use slm_router::routing::RoutingConfig;

fn row(prompt: &str, cloud: bool) -> Labelled {
    Labelled {
        prompt: prompt.into(),
        cloud,
    }
}

const EASY: &str = "What is the capital of France?";
const EASY2: &str = "hi there";
const HARD: &str = "Explain how CUDA warp divergence affects kernel throughput.";

#[test]
fn parse_skips_blank_lines_and_keeps_line_numbers() {
    let text = "{\"prompt\": \"a\", \"label\": \"local\", \"id\": 7}\n\n   \n{\"prompt\": \"b\", \"label\": \"maybe\"}\n";
    let err = parse(text).unwrap_err();
    assert!(err.contains("line 4") && err.contains("maybe"), "{err}");
    let ok = parse("\n{\"prompt\": \"a\", \"label\": \"cloud\"}\n\n").unwrap();
    assert_eq!((ok.len(), ok[0].cloud), (1, true));
}

#[test]
fn parse_names_the_malformed_line() {
    let err = parse("{\"prompt\": \"a\", \"label\": \"local\"}\nnot json\n").unwrap_err();
    assert!(err.contains("line 2"), "{err}");
    let err = parse("{\"label\": \"local\"}").unwrap_err();
    assert!(err.contains("line 1") && err.contains("prompt"), "{err}");
}

#[test]
fn empty_or_cloud_only_input_is_an_error() {
    let cfg = RoutingConfig::default();
    assert!(threshold(&parse("\n\n").unwrap(), &cfg).is_err());
    let err = threshold(&[row(HARD, true)], &cfg).unwrap_err();
    assert!(err.contains("no local rows"), "{err}");
}

#[test]
fn threshold_is_highest_local_score_plus_margin() {
    let cfg = RoutingConfig::default();
    let rows = [row(EASY, false), row(EASY2, false), row(HARD, true)];
    let o = threshold(&rows, &cfg).unwrap();
    let max = [EASY, EASY2]
        .map(|p| calibrate::score(p, &cfg).unwrap())
        .into_iter()
        .fold(0.0f32, f32::max);
    assert!(o.threshold >= max + calibrate::MARGIN, "{o:?} {max}");
    assert!(o.threshold < max + calibrate::MARGIN + 0.001, "{o:?} {max}");
    assert_eq!((o.local, o.cloud, o.excluded), (2, 1, 0));
}

#[test]
fn long_prompts_are_excluded_and_all_long_locals_is_no_local_rows() {
    let cfg = RoutingConfig::default();
    let long = "word ".repeat(cfg.complex_prompt_tokens + 10);
    let o = threshold(
        &[row(EASY, false), row(&long, false), row(&long, true)],
        &cfg,
    )
    .unwrap();
    assert_eq!((o.local, o.cloud, o.excluded), (1, 0, 2));
    let err = threshold(&[row(&long, false), row(HARD, true)], &cfg).unwrap_err();
    assert!(err.contains("no local rows"), "{err}");
}

#[test]
fn score_ignores_a_disabled_threshold() {
    let off = RoutingConfig {
        classifier_threshold: 2.0,
        ..RoutingConfig::default()
    };
    assert_eq!(
        calibrate::score(HARD, &off),
        calibrate::score(HARD, &RoutingConfig::default())
    );
}

#[test]
fn render_notes_disabling_thresholds_few_rows_and_no_cloud_rows() {
    let o = Outcome {
        threshold: 1.013,
        local: 3,
        cloud: 0,
        excluded: 1,
        caught: 0,
    };
    let out = render(&o, 0.78);
    assert!(out.starts_with("classifier_threshold = 1.013"), "{out}");
    for want in [
        "disables the classifier",
        "only 3 local rows",
        "n/a",
        "0.780",
    ] {
        assert!(out.contains(want), "{want}: {out}");
    }
    let o = Outcome {
        threshold: 0.5,
        local: 40,
        cloud: 4,
        excluded: 0,
        caught: 3,
    };
    let out = render(&o, 0.78);
    assert!(
        out.contains("3 / 4 (75.0%)") && !out.contains("warning"),
        "{out}"
    );
}

#[tokio::test]
async fn calibrate_scores_prompts_as_the_router_logs_them() {
    let r = rig(Mode::Ok, Mode::Ok, |_| {}).await;
    for p in [EASY, HARD] {
        let (status, _) = post_chat(&r.base, &user(p), &[]).await;
        assert_eq!(status, 200);
    }
    let lines = log_lines(&r.dir.log());
    let cfg = RoutingConfig::default();
    for (line, p) in lines.iter().zip([EASY, HARD]) {
        let ours = calibrate::score(p, &cfg).unwrap();
        let rounded = (f64::from(ours) * 1000.0).round() / 1000.0;
        assert_eq!(line["classifier_score"], json!(rounded), "{p}");
    }
}
