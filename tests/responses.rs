//! `POST /v1/responses` (non-stream): passthrough, stateful routing, unsupported upstreams.
mod common;
use common::*;
use serde_json::{Value, json};

/// A trimmed real Ollama `/v1/responses` reply.
const RESP_BODY: &str = r#"{"id":"resp_1","object":"response","status":"completed","output":[{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"output_text","text":"Hi there.","annotations":[]}]}],"usage":{"input_tokens":27,"output_tokens":10,"total_tokens":37}}"#;
const OPENAI_404: &str = r#"{"error":{"message":"Previous response with id 'resp_x' not found.","type":"invalid_request_error","param":"previous_response_id","code":"previous_response_not_found"}}"#;

fn resp(extra: Value) -> Value {
    let mut r = json!({"model": "gpt-4.1", "instructions": "be brief", "input": "say hi",
                       "text": {"format": {"type": "text"}}});
    r.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    r
}

async fn post(base: &str, body: &Value, headers: &[(&str, &str)]) -> (u16, String) {
    let mut r = reqwest::Client::new()
        .post(format!("{base}/v1/responses"))
        .json(body);
    for (k, v) in headers {
        r = r.header(*k, *v);
    }
    let resp = r.send().await.unwrap();
    (resp.status().as_u16(), resp.text().await.unwrap())
}

fn code(body: &str) -> Value {
    serde_json::from_str::<Value>(body).unwrap()["error"]["code"].clone()
}

#[tokio::test]
async fn local_passthrough_rewrites_only_the_model_and_logs_real_usage() {
    let r = rig(Mode::Status(200, RESP_BODY), Mode::Ok, |_| {}).await;
    let body = resp(
        json!({"previous_response_id": null, "store": false, "tools": [],
                           "tool_choice": "none", "custom_field": 1}),
    );
    let (status, text) = post(&r.base, &body, &[]).await;
    assert_eq!((status, text.as_str()), (200, RESP_BODY));
    let seen = r.local.last();
    assert_eq!(seen.path, "/v1/responses");
    let mut want = body.clone();
    want["model"] = json!("llama-test");
    assert_eq!(seen.body, want);
    assert_eq!(r.cloud.count(), 0);
    let l = &log_lines(&r.dir.log())[0];
    assert_eq!(
        (
            &l["prompt_tokens"],
            &l["completion_tokens"],
            &l["tokens_estimated"]
        ),
        (&json!(27), &json!(10), &json!(false))
    );
}

#[tokio::test]
async fn stateful_request_goes_to_cloud_and_never_to_local() {
    let r = rig(
        Mode::Status(200, RESP_BODY),
        Mode::Status(200, RESP_BODY),
        |_| {},
    )
    .await;
    let (status, _) = post(
        &r.base,
        &resp(json!({"previous_response_id": "resp_0"})),
        &[],
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        (r.local.count(), r.cloud.last().path.as_str()),
        (0, "/v1/responses")
    );
    assert_eq!(log_lines(&r.dir.log())[0]["reason"], "stateful");
}

#[tokio::test]
async fn stateful_request_that_must_stay_local_is_a_400() {
    let r = rig(
        Mode::Status(200, RESP_BODY),
        Mode::Status(200, RESP_BODY),
        |_| {},
    )
    .await;
    let stateful = resp(json!({"conversation": "conv_1"}));
    let (status, body) = post(&r.base, &stateful, &[("x-router-policy", "local-only")]).await;
    assert_eq!(
        (status, code(&body)),
        (400, json!("stateful_requires_cloud"))
    );
    let mut forced = stateful.clone();
    forced["model"] = json!("slm-router/local");
    let (status, body) = post(&r.base, &forced, &[]).await;
    assert_eq!(
        (status, code(&body)),
        (400, json!("stateful_requires_cloud"))
    );
    assert_eq!((r.local.count(), r.cloud.count()), (0, 0));
}

#[tokio::test]
async fn local_5xx_fails_over_to_cloud_responses() {
    let r = rig(
        Mode::Status(503, "{}"),
        Mode::Status(200, RESP_BODY),
        |_| {},
    )
    .await;
    let (status, text) = post(&r.base, &resp(json!({})), &[]).await;
    assert_eq!((status, text.as_str()), (200, RESP_BODY));
    assert_eq!(r.cloud.last().path, "/v1/responses");
    assert_eq!(log_lines(&r.dir.log())[0]["failed_over"], true);
}

#[tokio::test]
async fn missing_route_is_a_501_and_never_fails_over() {
    let r = rig(Mode::Status(404, "404 page not found"), Mode::Ok, |_| {}).await;
    let (status, body) = post(&r.base, &resp(json!({})), &[]).await;
    assert_eq!((status, code(&body)), (501, json!("responses_unsupported")));
    assert!(body.contains("local upstream"), "{body}");
    assert_eq!(r.cloud.count(), 0);
    let l = &log_lines(&r.dir.log())[0];
    assert_eq!(
        (&l["status"], &l["outcome"]),
        (&json!(501), &json!("upstream_error"))
    );

    for (code_, text) in [(404, r#"{"detail":"Not Found"}"#), (405, "")] {
        let r = rig(Mode::Ok, Mode::Status(code_, text), |_| {}).await;
        let cloud = resp(json!({"model": "slm-router/cloud"}));
        let (status, body) = post(&r.base, &cloud, &[]).await;
        assert_eq!(
            (status, code(&body)),
            (501, json!("responses_unsupported")),
            "{code_}"
        );
        assert_eq!((r.cloud.count(), r.local.count()), (1, 0));
    }
}

#[tokio::test]
async fn a_501_does_not_pin_the_session() {
    let r = rig(Mode::Status(404, "404 page not found"), Mode::Ok, |_| {}).await;
    let sid = [("x-session-id", "s-501")];
    for _ in 0..2 {
        let (status, _) = post(&r.base, &resp(json!({})), &sid).await;
        assert_eq!(status, 501);
    }
    // A pin from the first 501 would make the second decision `pinned`.
    let reasons: Vec<Value> = log_lines(&r.dir.log())
        .iter()
        .map(|l| l["reason"].clone())
        .collect();
    assert_eq!(reasons, [json!("default"), json!("default")]);
}

#[tokio::test]
async fn an_openai_404_error_body_is_relayed_not_rewritten() {
    let r = rig(Mode::Ok, Mode::Status(404, OPENAI_404), |_| {}).await;
    let (status, text) = post(
        &r.base,
        &resp(json!({"previous_response_id": "resp_x"})),
        &[],
    )
    .await;
    assert_eq!((status, text.as_str()), (404, OPENAI_404));
}

#[tokio::test]
async fn auth_limits_and_validation_apply() {
    let r = rig(Mode::Status(200, RESP_BODY), Mode::Ok, |c| {
        c.server.api_key = Some("sk-router".into());
        c.server.max_body_bytes = 512;
    })
    .await;
    let (status, _) = post(&r.base, &resp(json!({})), &[]).await;
    assert_eq!(status, 401);
    let auth = [("authorization", "Bearer sk-router")];
    let (status, _) = post(&r.base, &resp(json!({})), &auth).await;
    assert_eq!(status, 200);
    let (status, _) = post(&r.base, &resp(json!({"input": "x".repeat(600)})), &auth).await;
    assert_eq!(status, 413);
    let (status, body) = post(&r.base, &json!({"model": "m", "input": 7}), &auth).await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(r.local.count(), 1);
}
