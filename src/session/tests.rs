use super::*;
use tokio::time::advance;

const TTL: Duration = Duration::from_secs(300);

#[tokio::test(start_paused = true)]
async fn pin_is_returned_until_ttl_expires() {
    let p = PinStore::new(TTL, 10);
    p.set("s", Target::Local);
    assert_eq!(p.get("s"), Some(Target::Local));
    advance(TTL + Duration::from_secs(1)).await;
    assert_eq!(p.get("s"), None);
}

#[tokio::test(start_paused = true)]
async fn each_request_refreshes_the_ttl() {
    let p = PinStore::new(TTL, 10);
    p.set("s", Target::Local);
    advance(Duration::from_secs(200)).await;
    p.set("s", Target::Local);
    advance(Duration::from_secs(200)).await;
    assert_eq!(p.get("s"), Some(Target::Local));
}

#[tokio::test(start_paused = true)]
async fn pin_escalates_to_cloud_but_never_back() {
    let p = PinStore::new(TTL, 10);
    p.set("s", Target::Local);
    p.set("s", Target::Cloud);
    assert_eq!(p.get("s"), Some(Target::Cloud));
    p.set("s", Target::Local);
    assert_eq!(p.get("s"), Some(Target::Cloud));
}

#[tokio::test(start_paused = true)]
async fn least_recently_used_pin_is_evicted_when_full() {
    let p = PinStore::new(TTL, 2);
    for id in ["a", "b", "c"] {
        p.set(id, Target::Local);
        advance(Duration::from_millis(1)).await;
    }
    assert_eq!(p.get("a"), None);
    assert_eq!(p.get("b"), Some(Target::Local));
    assert_eq!(p.get("c"), Some(Target::Local));
}

#[tokio::test(start_paused = true)]
async fn eviction_order_is_insertion_order_even_within_one_instant() {
    let p = PinStore::new(TTL, 2);
    for id in ["a", "b", "c"] {
        p.set(id, Target::Local);
    }
    assert_eq!(p.get("a"), None);
    assert_eq!(p.get("b"), Some(Target::Local));
}

#[tokio::test(start_paused = true)]
async fn refreshed_pin_survives_eviction_and_store_stays_bounded() {
    let p = PinStore::new(TTL, 2);
    p.set("a", Target::Local);
    p.set("b", Target::Local);
    p.set("a", Target::Local);
    p.set("c", Target::Local);
    assert_eq!(p.get("b"), None);
    assert_eq!(p.get("a"), Some(Target::Local));
    for i in 0..1000 {
        p.set("a", Target::Local);
        p.set(&format!("x{i}"), Target::Local);
    }
    assert!(p.len() <= 2);
}
