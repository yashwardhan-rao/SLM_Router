mod common;
use common::*;

#[tokio::test]
async fn local_stream_requests_usage_and_hides_the_usage_chunk_the_client_did_not_ask_for() {
    let r = rig(Mode::Sse(&[HELLO, USAGE]), Mode::Ok, |_| {}).await;
    let (status, ctype, body) = fetch(&r.base, &stream_req(), &[]).await;
    assert_eq!(status, 200);
    assert!(ctype.starts_with("text/event-stream"), "{ctype}");
    assert_eq!(body, sse(&[HELLO]));
    assert_eq!(r.local.last().body["model"], "llama-test");
    assert_eq!(r.local.last().body["stream"], true);
    assert_eq!(r.local.last().body["stream_options"]["include_usage"], true);

    let l = &log_lines_wait(&r.dir.log(), 1).await[0];
    assert_eq!(
        (
            l["stream"].as_bool(),
            l["target"].as_str(),
            l["outcome"].as_str()
        ),
        (Some(true), Some("local"), Some("ok"))
    );
    assert_eq!(
        (
            l["prompt_tokens"].as_u64(),
            l["completion_tokens"].as_u64(),
            l["tokens_estimated"].as_bool()
        ),
        (Some(7), Some(3), Some(false))
    );
    assert!((l["saved_usd"].as_f64().unwrap() - 0.000066).abs() < 1e-12);
}

#[tokio::test]
async fn missing_usage_falls_back_to_a_flagged_estimate() {
    let r = rig(Mode::Sse(&[HELLO]), Mode::Ok, |_| {}).await;
    fetch(&r.base, &stream_req(), &[]).await;
    let l = &log_lines_wait(&r.dir.log(), 1).await[0];
    // "hello world!" is 12 chars -> 3 estimated completion tokens.
    assert_eq!(
        (
            l["completion_tokens"].as_u64(),
            l["tokens_estimated"].as_bool()
        ),
        (Some(3), Some(true))
    );
}

#[tokio::test]
async fn cloud_stream_uses_the_server_side_key_and_rewritten_model() {
    let r = rig(Mode::Ok, Mode::Sse(&[HELLO, USAGE]), |_| {}).await;
    let (status, _, body) = fetch(
        &r.base,
        &forced_cloud(),
        &[("authorization", "Bearer client")],
    )
    .await;
    assert_eq!((status, body), (200, sse(&[HELLO])));
    assert_eq!(r.cloud.last().body["stream_options"]["include_usage"], true);
    assert_eq!(r.cloud.last().auth.as_deref(), Some("Bearer sk-cloud"));
    assert_eq!(r.cloud.last().body["model"], "cloud-test");
    assert_eq!(r.local.count(), 0);
}

#[tokio::test]
async fn usage_is_found_and_hidden_even_when_an_event_is_split_across_chunks() {
    let r = rig(Mode::SseSplit(&[HELLO, USAGE]), Mode::Ok, |_| {}).await;
    let (_, _, body) = fetch(&r.base, &stream_req(), &[]).await;
    assert_eq!(body, format!("data: {HELLO}\n\n"));
    let l = &log_lines_wait(&r.dir.log(), 1).await[0];
    assert_eq!(
        (
            l["prompt_tokens"].as_u64(),
            l["completion_tokens"].as_u64(),
            l["tokens_estimated"].as_bool()
        ),
        (Some(7), Some(3), Some(false))
    );
}

#[tokio::test]
async fn a_client_that_asked_for_usage_gets_the_stream_verbatim() {
    let r = rig(Mode::SseSplit(&[HELLO, USAGE]), Mode::Ok, |_| {}).await;
    let mut req = stream_req();
    req["stream_options"] = serde_json::json!({"include_usage": true, "x_extra": 1});
    let (_, _, body) = fetch(&r.base, &req, &[]).await;
    assert_eq!(
        body,
        format!(
            "data: {HELLO}

data: {USAGE}

"
        ),
        "bytes must pass through unchanged"
    );
    assert_eq!(r.local.last().body["stream_options"]["x_extra"], 1);
}

#[tokio::test]
async fn other_stream_options_keys_survive_the_usage_request() {
    let r = rig(Mode::Sse(&[HELLO, USAGE]), Mode::Ok, |_| {}).await;
    let mut req = stream_req();
    req["stream_options"] = serde_json::json!({"x_extra": "keep"});
    let (_, _, body) = fetch(&r.base, &req, &[]).await;
    assert_eq!(body, sse(&[HELLO]));
    let sent = r.local.last().body;
    assert_eq!(sent["stream_options"]["x_extra"], "keep");
    assert_eq!(sent["stream_options"]["include_usage"], true);
}

#[tokio::test]
async fn non_object_stream_options_is_rejected_before_any_upstream_call() {
    let r = rig(Mode::Sse(&[HELLO, USAGE]), Mode::Ok, |_| {}).await;
    let mut req = stream_req();
    req["stream_options"] = serde_json::json!("x");
    let (status, _, body) = fetch(&r.base, &req, &[]).await;
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("stream_options"), "{body}");
    assert_eq!(r.local.count() + r.cloud.count(), 0);
}

#[tokio::test]
async fn failover_to_cloud_also_requests_and_hides_usage() {
    let r = rig(Mode::Status(503, "{}"), Mode::Sse(&[HELLO, USAGE]), |_| {}).await;
    let (status, _, body) = fetch(&r.base, &stream_req(), &[]).await;
    assert_eq!((status, body), (200, sse(&[HELLO])));
    assert_eq!(r.cloud.last().body["stream_options"]["include_usage"], true);
    let l = &log_lines_wait(&r.dir.log(), 1).await[0];
    assert_eq!(l["tokens_estimated"].as_bool(), Some(false));
}

#[tokio::test]
async fn include_usage_false_or_null_options_still_get_exact_counts_and_no_usage_chunk() {
    for opts in [
        serde_json::json!({"include_usage": false}),
        serde_json::json!(null),
    ] {
        let r = rig(Mode::Sse(&[HELLO, USAGE]), Mode::Ok, |_| {}).await;
        let mut req = stream_req();
        req["stream_options"] = opts.clone();
        let (status, _, body) = fetch(&r.base, &req, &[]).await;
        assert_eq!((status, body), (200, sse(&[HELLO])), "{opts}");
        assert_eq!(r.local.last().body["stream_options"]["include_usage"], true);
    }
}
