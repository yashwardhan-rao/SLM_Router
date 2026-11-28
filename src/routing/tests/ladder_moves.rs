//! Pins and failover moving up a `[[tier]]` ladder.

use super::ladder::*;
use super::*;

#[test]
fn a_pin_holds_and_escalates_to_a_tier_that_can_serve() {
    let a = Policy::Auto;
    assert_eq!(go(three(), &text("hi"), a, Some(1)), (1, Reason::Pinned));
    assert_eq!(go(three(), &text("hi"), a, Some(2)), (2, Reason::Pinned));
    assert_eq!(go(three(), &stateful(), a, Some(1)), (2, Reason::Stateful));
    // Tools fit tier 0 but not the pinned tier 1: escalate past the pin to tier 2.
    let tiers = vec![
        t(true, true, None, false, false),
        t(true, false, None, false, false),
        t(false, true, None, true, true),
    ];
    assert_eq!(go(tiers, &tools(), a, Some(1)), (2, Reason::Pinned));
    // Nothing at or above the pin fits the context: the pin is ignored for this request.
    let shrinking = vec![
        t(true, false, None, false, false),
        t(false, true, Some(1024), true, true),
    ];
    assert_eq!(
        go(shrinking, &tokens(2000), a, Some(1)),
        (0, Reason::Default)
    );
}

#[test]
fn next_up_skips_tiers_that_cannot_serve_and_respects_policy() {
    let tiers = vec![
        t(true, false, None, false, false),
        t(true, false, Some(1024), false, false),
        t(false, false, None, false, false),
        t(false, true, None, true, true),
    ];
    let (a, lo) = (Policy::Auto, Policy::LocalOnly);
    assert_eq!(next_up(&tiers, 0, &text("hi"), a), Some(1));
    assert_eq!(next_up(&tiers, 0, &tokens(2000), a), Some(2));
    assert_eq!(next_up(&tiers, 0, &tools(), a), Some(3));
    assert_eq!(next_up(&tiers, 0, &stateful(), a), Some(3));
    assert_eq!(next_up(&tiers, 0, &tokens(2000), lo), None);
    assert_eq!(next_up(&tiers, 3, &text("hi"), a), None);
}
