mod common;
use common::*;
use serde_json::Value;

#[tokio::test]
async fn stream_fails_over_to_cloud_when_local_is_down_before_the_first_byte() {
    for local_down in ["refused", "5xx", "hang"] {
        let (mode, dead) = match local_down {
            "5xx" => (Mode::Status(503, "{}"), false),
            "hang" => (Mode::Hang, false),
            _ => (Mode::Ok, true),
        };
        let r = rig(mode, Mode::Sse(&[HELLO, USAGE]), |c| {
            if dead {
                c.local.base_url = dead_url();
            }
        })
        .await;
        let (status, _, body) = fetch(&r.base, &stream_req(), &[]).await;
        assert_eq!((status, body), (200, sse(&[HELLO])), "local {local_down}");
        let l = &log_lines_wait(&r.dir.log(), 1).await[0];
        assert_eq!(
            (l["failed_over"].as_bool(), l["reason"].as_str()),
            (Some(true), Some("local_unavailable")),
            "local {local_down}"
        );
    }
}

#[tokio::test]
async fn local_only_stream_with_local_down_is_a_503_json_error() {
    let r = rig(Mode::Ok, Mode::Ok, |c| c.local.base_url = dead_url()).await;
    let (status, ctype, body) =
        fetch(&r.base, &stream_req(), &[("x-router-policy", "local-only")]).await;
    assert_eq!(status, 503);
    assert!(
        ctype.starts_with("application/json") && body.contains("\"error\""),
        "{ctype} {body}"
    );
    assert_eq!(r.cloud.count(), 0);
}

#[tokio::test]
async fn upstream_error_on_a_stream_request_passes_through_unchanged() {
    let r = rig(
        Mode::Ok,
        Mode::Status(429, r#"{"error":"slow down"}"#),
        |_| {},
    )
    .await;
    let (status, _, body) = fetch(&r.base, &forced_cloud(), &[]).await;
    assert_eq!((status, body.as_str()), (429, r#"{"error":"slow down"}"#));
    let l = &log_lines_wait(&r.dir.log(), 1).await[0];
    assert_eq!(
        (l["status"].as_u64(), l["outcome"].as_str()),
        (Some(429), Some("upstream_error"))
    );
}

#[tokio::test]
async fn upstream_dying_mid_stream_ends_with_an_error_event_and_a_partial_log_record() {
    let r = rig(Mode::SseDies, Mode::Ok, |_| {}).await;
    let (status, _, body) = fetch(&r.base, &stream_req(), &[]).await;
    assert_eq!(status, 200, "headers were already sent");
    assert!(body.contains("partial text"), "{body}");
    let last = body.trim().rsplit("data: ").next().unwrap();
    let err: Value = serde_json::from_str(last.trim()).unwrap();
    assert_eq!(err["error"]["code"], "stream_interrupted");
    assert!(err["error"]["message"].is_string() && err["error"]["type"].is_string());

    let l = &log_lines_wait(&r.dir.log(), 1).await[0];
    assert_eq!(
        (
            l["outcome"].as_str(),
            l["stream"].as_bool(),
            l["tokens_estimated"].as_bool()
        ),
        (Some("stream_error"), Some(true), Some(true))
    );
    assert_eq!(l["completion_tokens"], 3, "tokens counted so far are kept");
}

#[tokio::test]
async fn client_disconnect_cancels_the_upstream_and_logs_a_partial_record() {
    let r = rig(Mode::SseForever, Mode::Ok, |_| {}).await;
    let mut resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", r.base))
        .json(&stream_req())
        .send()
        .await
        .unwrap();
    assert!(resp.chunk().await.unwrap().is_some(), "stream should start");
    drop(resp);

    eventually("upstream request dropped", || r.local.was_dropped()).await;
    let l = &log_lines_wait(&r.dir.log(), 1).await[0];
    assert_eq!(l["outcome"], "client_disconnect");
    assert_eq!(l["stream"], true);
}
