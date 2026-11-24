use slm_router::circuit::Breaker;
use std::time::{Duration, Instant};

const COOL: Duration = Duration::from_millis(1_000);

fn ms(t0: Instant, n: u64) -> Instant {
    t0 + Duration::from_millis(n)
}

/// A breaker (threshold 2) that has just opened at `t0`.
fn opened() -> (Breaker, Instant) {
    let (b, t0) = (Breaker::new(2, COOL), Instant::now());
    b.record(false, t0);
    b.record(false, t0);
    (b, t0)
}

#[test]
fn stays_closed_below_the_threshold() {
    let (b, t0) = (Breaker::new(2, COOL), Instant::now());
    b.record(false, t0);
    assert!(b.allow(t0));
}

#[test]
fn opens_at_two_consecutive_failures() {
    let (b, t0) = opened();
    assert!(!b.allow(t0));
    assert!(!b.allow(ms(t0, 999)));
}

#[test]
fn a_success_resets_the_count() {
    let (b, t0) = (Breaker::new(2, COOL), Instant::now());
    b.record(false, t0);
    b.record(true, t0);
    b.record(false, t0);
    assert!(b.allow(t0));
}

#[test]
fn exactly_one_probe_after_the_cooldown() {
    let (b, t0) = opened();
    assert!(b.allow(ms(t0, 1_000)), "first caller after cooldown probes");
    assert!(!b.allow(ms(t0, 1_001)), "second caller waits for the probe");
}

#[test]
fn a_probe_success_closes_it() {
    let (b, t0) = opened();
    assert!(b.allow(ms(t0, 1_000)));
    b.record(true, ms(t0, 1_010));
    assert!(b.allow(ms(t0, 1_011)));
    assert!(b.allow(ms(t0, 1_012)));
}

#[test]
fn a_probe_failure_reopens_it_with_a_new_until() {
    let (b, t0) = opened();
    assert!(b.allow(ms(t0, 1_000)));
    b.record(false, ms(t0, 1_500));
    assert!(!b.allow(ms(t0, 2_400)), "reopened until 2500");
    assert!(b.allow(ms(t0, 2_500)));
}

#[test]
fn an_abandoned_probe_lease_expires_and_allows_a_new_probe() {
    let (b, t0) = opened();
    assert!(
        b.allow(ms(t0, 1_000)),
        "probe whose client hangs up, never recorded"
    );
    assert!(!b.allow(ms(t0, 1_999)));
    assert!(b.allow(ms(t0, 2_000)), "lease expired: a new probe");
    assert!(!b.allow(ms(t0, 2_001)));
}

#[test]
fn a_late_failure_while_open_does_not_extend_until() {
    let (b, t0) = opened();
    b.record(false, ms(t0, 900));
    assert!(b.allow(ms(t0, 1_000)));
}

#[test]
fn threshold_zero_never_opens() {
    let (b, t0) = (Breaker::new(0, COOL), Instant::now());
    for _ in 0..10 {
        b.record(false, t0);
    }
    assert!(b.allow(t0));
}
