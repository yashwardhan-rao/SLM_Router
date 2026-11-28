//! Choosing a threshold from scored, labelled prompts. Pure: points in, metrics out.

/// One labelled prompt as rule 5 sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    /// Classifier probability.
    pub p: f32,
    /// Keyword cues alone send it to the cloud, whatever the threshold.
    pub cued: bool,
    /// Labelled `cloud`: the local model's answer was not good enough.
    pub cloud: bool,
}

/// What a threshold does to the labelled file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub threshold: f32,
    pub local: usize,
    /// Local rows the router keeps local at this threshold.
    pub local_kept: usize,
    pub cloud: usize,
    /// Cloud rows the router sends to the cloud (cues or classifier).
    pub cloud_caught: usize,
}

impl Metrics {
    /// Local retention (local recall).
    pub fn retention(&self) -> f64 {
        ratio(self.local_kept, self.local).unwrap_or(1.0)
    }
    /// Share of cloud rows wrongly kept local; None without cloud rows.
    pub fn false_local(&self) -> Option<f64> {
        ratio(self.cloud - self.cloud_caught, self.cloud)
    }
    pub fn accuracy(&self) -> f64 {
        ratio(self.local_kept + self.cloud_caught, self.local + self.cloud).unwrap_or(0.0)
    }
    /// F1 with `cloud` as the positive class: what rule 5 is trying to detect.
    pub fn f1(&self) -> f64 {
        let fp = self.local - self.local_kept;
        let fn_ = self.cloud - self.cloud_caught;
        ratio(2 * self.cloud_caught, 2 * self.cloud_caught + fp + fn_).unwrap_or(0.0)
    }
}

fn ratio(n: usize, d: usize) -> Option<f64> {
    (d > 0).then(|| n as f64 / d as f64)
}

/// What the suggested threshold optimises.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    /// Best F1 on the cloud class.
    F1,
    /// Catch the most cloud rows while keeping at least this share of local rows local.
    MinLocalRecall(f64),
    /// Keep the most local rows local while wrongly keeping at most this share of cloud rows.
    MaxFalseLocal(f64),
}

/// Rule 5 at threshold `t`: cloud when cued or when the probability reaches `t`.
pub fn eval(points: &[Point], t: f32) -> Metrics {
    let goes_cloud = |x: &Point| x.cued || x.p >= t;
    let local = points.iter().filter(|x| !x.cloud).count();
    Metrics {
        threshold: t,
        local,
        local_kept: points.iter().filter(|x| !x.cloud && !goes_cloud(x)).count(),
        cloud: points.len() - local,
        cloud_caught: points.iter().filter(|x| x.cloud && goes_cloud(x)).count(),
    }
}

/// A threshold strictly above `lo` (kept local) and at most `hi` (sent to cloud), on 3 decimals.
fn between(lo: Option<f32>, hi: Option<f32>) -> f32 {
    let r3 = |v: f32| (v * 1000.0).round() / 1000.0;
    match (lo, hi) {
        (Some(lo), None) => ((lo + super::MARGIN) * 1000.0).ceil() / 1000.0,
        (None, Some(hi)) => ((hi * 1000.0).floor() / 1000.0).max(0.001),
        (Some(lo), Some(hi)) => Some(r3((lo + hi) / 2.0))
            .filter(|&t| lo < t && t <= hi)
            .unwrap_or(hi),
        (None, None) => 1.0 + super::MARGIN,
    }
}

/// Every distinct cut through the sorted scores, as the metrics at its threshold.
pub fn candidates(points: &[Point]) -> Vec<Metrics> {
    let mut ps: Vec<f32> = points.iter().map(|x| x.p).collect();
    ps.sort_by(f32::total_cmp);
    ps.dedup();
    (0..=ps.len())
        .map(|k| between(k.checked_sub(1).map(|i| ps[i]), ps.get(k).copied()))
        .map(|t| eval(points, t))
        .collect()
}

pub fn pick(points: &[Point], target: Target) -> Result<Metrics, String> {
    let all = candidates(points);
    let best = |ok: &dyn Fn(&Metrics) -> bool, key: &dyn Fn(&Metrics) -> (f64, f64)| {
        all.iter().filter(|m| ok(m)).copied().max_by(|a, b| {
            let (ka, kb) = (key(a), key(b));
            ka.0.total_cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
        })
    };
    let eps = 1e-9;
    match target {
        Target::F1 => {
            if points.iter().all(|x| !x.cloud) {
                return Err("F1 needs at least one prompt labelled \"cloud\"; use --min-local-recall instead".into());
            }
            best(&|_| true, &|m| (m.f1(), m.retention()))
        }
        Target::MinLocalRecall(r) => best(&|m| m.retention() >= r - eps, &|m| {
            (m.cloud_caught as f64, m.retention())
        }),
        Target::MaxFalseLocal(f) => {
            best(&|m| m.false_local().unwrap_or(0.0) <= f + eps, &|m| {
                (m.retention(), m.cloud_caught as f64)
            })
        }
    }
    .ok_or_else(|| {
        let top = eval(points, f32::INFINITY).retention();
        format!(
            "no threshold reaches {target:?}; the highest local retention possible is {:.1}% (cues alone send some local rows to cloud)",
            top * 100.0
        )
    })
}
