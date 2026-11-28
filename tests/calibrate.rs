mod common;

use common::*;
use serde_json::json;
use slm_router::calibrate::{self, Labelled, Metrics, Outcome, Target, parse, render, threshold};
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
    assert!(
        threshold(
            &parse(
                "

"
            )
            .unwrap(),
            &cfg,
            Target::F1
        )
        .is_err()
    );
    let err = threshold(&[row(HARD, true)], &cfg, Target::F1).unwrap_err();
    assert!(err.contains("no local rows"), "{err}");
}

#[test]
fn suggestion_comes_from_real_scores_and_matches_its_metrics() {
    let cfg = RoutingConfig::default();
    let rows = [row(EASY, false), row(EASY2, false), row(HARD, true)];
    let o = threshold(&rows, &cfg, Target::F1).unwrap();
    let (s, c) = (o.suggested, o.current);
    assert_eq!((s.local, s.cloud, o.excluded), (2, 1, 0));
    assert_eq!(
        (c.threshold, c.local, c.cloud),
        (cfg.classifier_threshold, 2, 1)
    );
    // The strict row is the old rule: just above the highest local score.
    let max = [EASY, EASY2]
        .map(|p| calibrate::score(p, &cfg).unwrap())
        .into_iter()
        .fold(0.0f32, f32::max);
    assert!(o.strict.threshold > max, "{o:?} {max}");
    assert_eq!(o.strict.local_kept, 2);
}

#[test]
fn long_prompts_are_excluded_and_all_long_locals_is_no_local_rows() {
    let cfg = RoutingConfig::default();
    let long = "word ".repeat(cfg.complex_prompt_tokens + 10);
    let rows = [row(EASY, false), row(&long, false), row(&long, true)];
    let o = threshold(&rows, &cfg, Target::MinLocalRecall(1.0)).unwrap();
    assert_eq!(
        (o.suggested.local, o.suggested.cloud, o.excluded),
        (1, 0, 2)
    );
    let err = threshold(&[row(&long, false), row(HARD, true)], &cfg, Target::F1).unwrap_err();
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

fn m(t: f32, local: usize, kept: usize, cloud: usize, caught: usize) -> Metrics {
    Metrics {
        threshold: t,
        local,
        local_kept: kept,
        cloud,
        cloud_caught: caught,
    }
}

#[test]
fn render_prints_the_table_before_the_suggestion() {
    let o = Outcome {
        target: Target::MinLocalRecall(0.95),
        suggested: m(0.512, 40, 38, 10, 9),
        current: m(0.781, 40, 40, 10, 5),
        strict: m(0.99, 40, 40, 10, 0),
        excluded: 1,
    };
    let out = render(&o);
    let lines: Vec<&str> = out.lines().collect();
    assert!(
        lines[0].contains("threshold") && lines[0].contains("false local"),
        "{out}"
    );
    for (line, want) in lines[1..4].iter().zip([
        ["suggested (kept>=95%)", "0.512", "95.0%", "10.0%", "94.0%"],
        ["current", "0.781", "100.0%", "50.0%", "90.0%"],
        ["strict (keep all)", "0.990", "100.0%", "100.0%", "80.0%"],
    ]) {
        for w in want {
            assert!(line.contains(w), "{w} in {line:?}");
        }
    }
    assert!(
        out.contains(
            "
classifier_threshold = 0.512
"
        ),
        "{out}"
    );
    assert!(
        out.contains("40 local, 10 cloud (1 over") && !out.contains("warning"),
        "{out}"
    );
}

#[test]
fn render_notes_disabling_thresholds_few_rows_and_no_cloud_rows() {
    let o = Outcome {
        target: Target::MinLocalRecall(1.0),
        suggested: m(1.013, 3, 3, 0, 0),
        current: m(0.78, 3, 3, 0, 0),
        strict: m(1.013, 3, 3, 0, 0),
        excluded: 0,
    };
    let out = render(&o);
    for want in ["disables the classifier", "only 3 local rows", "n/a"] {
        assert!(out.contains(want), "{want}: {out}");
    }
}

#[test]
fn fraction_accepts_0_to_1_only() {
    assert_eq!(calibrate::fraction("0.95"), Ok(0.95));
    for bad in ["1.5", "-0.1", "x", "NaN"] {
        assert!(calibrate::fraction(bad).is_err(), "{bad}");
    }
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
