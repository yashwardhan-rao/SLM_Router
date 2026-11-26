use crate::routing::classifier::{decode, model, prob};
use serde::Deserialize;

#[derive(Deserialize)]
struct Row {
    text: String,
    est_tokens: usize,
    cue_score: u32,
    prob: f32,
}

#[test]
fn matches_the_python_trainer() {
    let rows: Vec<Row> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/classifier_parity.json"
    ))
    .unwrap();
    assert_eq!(rows.len(), 50);
    for r in rows {
        let got = prob(&r.text, r.est_tokens, r.cue_score);
        assert!(
            (got - r.prob).abs() < 1e-3,
            "{:?}: rust {got} vs python {}",
            r.text,
            r.prob
        );
    }
}

#[test]
fn embedded_model_has_a_usable_threshold() {
    let t = model().threshold;
    assert!(t > 0.0 && t.is_finite(), "threshold {t}");
}

#[test]
fn decode_rejects_bad_magic_and_wrong_size() {
    let bin = include_bytes!("../../../assets/rule5.bin");
    let mut bad = bin.to_vec();
    bad[0] = b'X';
    let bad: &'static [u8] = bad.leak();
    assert!(decode(bad).unwrap_err().contains("magic"));
    assert!(decode(&bin[..bin.len() - 1]).unwrap_err().contains("bytes"));
    assert!(decode(bin).is_ok());
}

#[test]
fn probabilities_stay_in_range_for_odd_input() {
    for t in ["", "  ?! ", "東京", &"word ".repeat(2000)] {
        let p = prob(t, t.len(), 0);
        assert!((0.0..=1.0).contains(&p), "{t:?}: {p}");
    }
}

/// Budget: under 50 µs per 2 KB prompt. Only meaningful with optimisations on.
#[cfg(not(debug_assertions))]
#[test]
fn scores_a_2kb_prompt_in_under_50us() {
    let text = "How do I tune Postgres autovacuum for a write-heavy table? ".repeat(35);
    assert!(text.len() >= 2048);
    let start = std::time::Instant::now();
    let mut sink = 0.0;
    for _ in 0..1000 {
        sink += prob(std::hint::black_box(&text), 500, 1);
    }
    let per = start.elapsed() / 1000;
    assert!(sink > 0.0);
    assert!(per.as_micros() < 50, "{per:?} per prompt");
}
