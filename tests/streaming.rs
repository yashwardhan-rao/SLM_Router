mod common;
use common::*;

#[tokio::test]
async fn local_stream_is_relayed_verbatim_and_usage_is_read_from_the_final_chunk() {
    let r = rig(Mode::Sse(&[HELLO, USAGE]), Mode::Ok, |_| {}).await;
    let (status, ctype, body) = fetch(&r.base, &stream_req(), &[]).await;
    assert_eq!(status, 200);
    assert!(ctype.starts_with("text/event-stream"), "{ctype}");
    assert_eq!(body, sse(&[HELLO, USAGE]));
    assert_eq!(r.local.last().body["model"], "llama-test");
    assert_eq!(r.local.last().body["stream"], true);

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
    assert_eq!((status, body), (200, sse(&[HELLO, USAGE])));
    assert_eq!(r.cloud.last().auth.as_deref(), Some("Bearer sk-cloud"));
    assert_eq!(r.cloud.last().body["model"], "cloud-test");
    assert_eq!(r.local.count(), 0);
}

#[tokio::test]
async fn usage_is_found_even_when_an_event_is_split_across_chunks() {
    let r = rig(Mode::SseSplit(&[HELLO, USAGE]), Mode::Ok, |_| {}).await;
    let (_, _, body) = fetch(&r.base, &stream_req(), &[]).await;
    assert_eq!(
        body,
        format!("data: {HELLO}\n\ndata: {USAGE}\n\n"),
        "bytes must pass through unchanged"
    );
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
