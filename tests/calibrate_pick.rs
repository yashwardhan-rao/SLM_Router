//! The threshold picker on hand-made scores: one noisy row must not drag the threshold.

use slm_router::calibrate::{Point, Target, eval, pick};

fn pt(p: f32, cloud: bool) -> Point {
    Point {
        p,
        cued: false,
        cloud,
    }
}

/// 20 local rows scoring 0.10..0.29 and 20 cloud rows scoring 0.70..0.89: a clean gap.
fn clean() -> Vec<Point> {
    let mut v: Vec<Point> = (0..20).map(|i| pt(0.10 + i as f32 * 0.01, false)).collect();
    v.extend((0..20).map(|i| pt(0.70 + i as f32 * 0.01, true)));
    v
}

fn in_gap(t: f32) -> bool {
    0.29 < t && t <= 0.70
}

#[test]
fn a_clean_file_puts_every_target_in_the_gap() {
    let pts = clean();
    for target in [
        Target::F1,
        Target::MinLocalRecall(0.95),
        Target::MaxFalseLocal(0.05),
    ] {
        let m = pick(&pts, target).unwrap();
        assert!(in_gap(m.threshold), "{target:?}: {m:?}");
        assert_eq!((m.local_kept, m.cloud_caught), (20, 20), "{target:?}");
    }
}

#[test]
fn one_local_outlier_does_not_drag_the_threshold_up() {
    let mut pts = clean();
    pts.push(pt(0.97, false)); // an easy prompt the classifier scores as very hard
    // The old rule keeps every local row, so it lands above the outlier and keeps all cloud rows local.
    let strict = pick(&pts, Target::MinLocalRecall(1.0)).unwrap();
    assert!(strict.threshold > 0.97, "{strict:?}");
    assert_eq!(strict.false_local(), Some(1.0));
    for target in [Target::F1, Target::MinLocalRecall(0.95)] {
        let m = pick(&pts, target).unwrap();
        assert!(in_gap(m.threshold), "{target:?}: {m:?}");
        assert_eq!((m.local_kept, m.cloud_caught), (20, 20), "{target:?}");
        assert_eq!(m.false_local(), Some(0.0));
        assert!((m.retention() - 20.0 / 21.0).abs() < 1e-9);
    }
}

#[test]
fn one_mislabelled_cloud_row_does_not_drag_the_threshold_down() {
    let mut pts = clean();
    pts.push(pt(0.05, true)); // labelled cloud, but scores below every easy prompt
    for target in [Target::F1, Target::MaxFalseLocal(0.05)] {
        let m = pick(&pts, target).unwrap();
        assert!(in_gap(m.threshold), "{target:?}: {m:?}");
        assert_eq!((m.local_kept, m.cloud_caught), (20, 20), "{target:?}");
    }
    // A zero false-local ceiling has to send every local row to cloud to catch it.
    let zero = pick(&pts, Target::MaxFalseLocal(0.0)).unwrap();
    assert!(zero.threshold <= 0.05 && zero.local_kept == 0, "{zero:?}");
}

#[test]
fn the_reported_metrics_are_those_of_the_reported_threshold() {
    let mut pts = clean();
    pts.extend([
        pt(0.97, false),
        pt(0.05, true),
        pt(0.5, false),
        pt(0.5, true),
    ]);
    for target in [
        Target::F1,
        Target::MinLocalRecall(0.9),
        Target::MaxFalseLocal(0.1),
    ] {
        let m = pick(&pts, target).unwrap();
        assert_eq!(m, eval(&pts, m.threshold), "{target:?}");
        assert_eq!(m.threshold, (m.threshold * 1000.0).round() / 1000.0);
    }
}

#[test]
fn a_recall_floor_beyond_what_cues_allow_is_an_error() {
    let mut pts = clean();
    pts.push(Point {
        p: 0.1,
        cued: true,
        cloud: false,
    });
    let err = pick(&pts, Target::MinLocalRecall(1.0)).unwrap_err();
    assert!(err.contains("95.2%"), "{err}");
    assert!(pick(&pts, Target::MinLocalRecall(0.95)).is_ok());
}

#[test]
fn without_cloud_rows_f1_errors_and_a_recall_floor_keeps_all_local() {
    let pts: Vec<Point> = (0..5).map(|i| pt(0.2 + i as f32 * 0.1, false)).collect();
    let err = pick(&pts, Target::F1).unwrap_err();
    assert!(err.contains("--min-local-recall"), "{err}");
    let m = pick(&pts, Target::MinLocalRecall(1.0)).unwrap();
    assert_eq!((m.local_kept, m.false_local()), (5, None));
    assert!((m.threshold - 0.62).abs() < 1e-6, "{m:?}"); // highest score + 0.02
}

#[test]
fn ties_are_never_split() {
    let pts = vec![pt(0.4, false), pt(0.4, true), pt(0.4, false), pt(0.9, true)];
    let m = pick(&pts, Target::F1).unwrap();
    let below = pts.iter().filter(|x| x.p < m.threshold).count();
    assert!(below == 0 || below == 3, "{m:?}");
}
