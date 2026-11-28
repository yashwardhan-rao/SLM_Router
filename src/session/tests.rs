use super::*;
use tokio::time::advance;

const TTL: Duration = Duration::from_secs(300);

#[tokio::test(start_paused = true)]
async fn pin_is_returned_until_ttl_expires() {
    let p = PinStore::new(TTL, 10);
    p.set("s", 0);
    assert_eq!(p.get("s"), Some(0));
    advance(TTL + Duration::from_secs(1)).await;
    assert_eq!(p.get("s"), None);
}

#[tokio::test(start_paused = true)]
async fn each_request_refreshes_the_ttl() {
    let p = PinStore::new(TTL, 10);
    p.set("s", 0);
    advance(Duration::from_secs(200)).await;
    p.set("s", 0);
    advance(Duration::from_secs(200)).await;
    assert_eq!(p.get("s"), Some(0));
}

#[tokio::test(start_paused = true)]
async fn pin_escalates_up_the_ladder_but_never_back() {
    let p = PinStore::new(TTL, 10);
    p.set("s", 0);
    p.set("s", 1);
    assert_eq!(p.get("s"), Some(1));
    p.set("s", 0);
    assert_eq!(p.get("s"), Some(1));
}

#[tokio::test(start_paused = true)]
async fn least_recently_used_pin_is_evicted_when_full() {
    let p = PinStore::new(TTL, 2);
    for id in ["a", "b", "c"] {
        p.set(id, 0);
        advance(Duration::from_millis(1)).await;
    }
    assert_eq!(p.get("a"), None);
    assert_eq!(p.get("b"), Some(0));
    assert_eq!(p.get("c"), Some(0));
}

#[tokio::test(start_paused = true)]
async fn eviction_order_is_insertion_order_even_within_one_instant() {
    let p = PinStore::new(TTL, 2);
    for id in ["a", "b", "c"] {
        p.set(id, 0);
    }
    assert_eq!(p.get("a"), None);
    assert_eq!(p.get("b"), Some(0));
}

#[tokio::test(start_paused = true)]
async fn refreshed_pin_survives_eviction_and_store_stays_bounded() {
    let p = PinStore::new(TTL, 2);
    p.set("a", 0);
    p.set("b", 0);
    p.set("a", 0);
    p.set("c", 0);
    assert_eq!(p.get("b"), None);
    assert_eq!(p.get("a"), Some(0));
    for i in 0..1000 {
        p.set("a", 0);
        p.set(&format!("x{i}"), 0);
    }
    assert!(p.len() <= 2);
}
