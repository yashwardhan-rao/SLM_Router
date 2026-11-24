//! Feature extraction and session fingerprinting for `/v1/responses` bodies.
use serde_json::{Value, json};
use slm_router::{
    schema::{features, responses_view, stateful, text_of},
    session::session_id,
};

fn req(extra: Value) -> Value {
    let mut r = json!({"model": "gpt-4.1", "input": "hello"});
    r.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    r
}

fn caps(v: &Value) -> bool {
    features(&responses_view(v).unwrap()).needs_capabilities
}

#[test]
fn sdk_nulls_and_defaults_trigger_nothing() {
    let v = req(json!({
        "previous_response_id": null, "conversation": null, "store": false,
        "background": false, "tools": [], "tool_choice": "none",
        "text": {"format": {"type": "text"}}, "instructions": null,
    }));
    assert!(!caps(&v));
    assert!(!stateful(&v));
    assert!(!stateful(&req(
        json!({"previous_response_id": "", "conversation": ""})
    )));
}

#[test]
fn each_capability_trigger_needs_capabilities() {
    let img = json!([{"role": "user", "content": [{"type": "input_image", "image_url": "x"}]}]);
    let file = json!([{"role": "user", "content": [{"type": "input_file", "file_id": "f"}]}]);
    let cases = [
        json!({"tools": [{"type": "function", "name": "f"}]}),
        json!({"tool_choice": "required"}),
        json!({"tool_choice": {"type": "function", "name": "f"}}),
        json!({"text": {"format": {"type": "json_schema", "name": "s", "schema": {}}}}),
        json!({"input": img}),
        json!({"input": file}),
        json!({"input": [{"type": "input_image", "image_url": "x"}]}),
    ];
    for c in cases {
        assert!(caps(&req(c.clone())), "{c}");
        assert!(!stateful(&req(c.clone())), "{c}");
    }
}

#[test]
fn each_stateful_trigger_is_stateful() {
    let cases = [
        json!({"previous_response_id": "resp_1"}),
        json!({"conversation": "conv_1"}),
        json!({"conversation": {"id": "conv_1"}}),
        json!({"store": true}),
        json!({"background": true}),
        json!({"input": [{"type": "item_reference", "id": "msg_1"}]}),
    ];
    for c in cases {
        assert!(stateful(&req(c.clone())), "{c}");
    }
}

#[test]
fn mixed_items_count_every_text_and_skip_reasoning() {
    let v = req(json!({
        "instructions": "be brief",                                                  // 8
        "input": [
            {"role": "developer", "content": "dev rules"},                           // 9
            {"type": "message", "role": "user",
             "content": [{"type": "input_text", "text": "first question"}]},         // 14
            {"role": "assistant", "content": [{"type": "output_text", "text": "ok"}]}, // 2
            {"type": "function_call", "call_id": "c", "name": "f", "arguments": "{\"a\":1}"}, // 7
            {"type": "function_call_output", "call_id": "c", "output": "result"},   // 6
            {"type": "reasoning", "summary": [{"type": "summary_text", "text": "LONG HIDDEN THOUGHT"}]},
            {"role": "user", "content": "last one"},                                // 8
        ],
    }));
    let f = features(&responses_view(&v).unwrap());
    assert_eq!(f.est_prompt_tokens, (8 + 9 + 14 + 2 + 7 + 6 + 8) / 4);
    assert_eq!(f.last_user_text, "last one");
    assert!(!f.needs_capabilities);
}

#[test]
fn equivalent_inputs_share_a_fingerprint_and_later_turns_do_not_change_it() {
    let sid = |input: Value| {
        let v = req(json!({"instructions": "sys", "input": input}));
        session_id(None, &responses_view(&v).unwrap()).0
    };
    let base = sid(json!("hi"));
    assert_eq!(base, sid(json!([{"role": "user", "content": "hi"}])));
    assert_eq!(
        base,
        sid(json!([{"role": "user", "content": [{"type": "input_text", "text": "hi"}]}]))
    );
    assert_eq!(
        base,
        sid(
            json!([{"role": "user", "content": "hi"}, {"role": "assistant", "content": "yo"},
                   {"role": "user", "content": "more"}])
        )
    );
    assert_ne!(base, sid(json!("other")));
}

#[test]
fn malformed_bodies_are_rejected() {
    for bad in [
        json!({"model": "m"}),
        json!({"model": "m", "input": 3}),
        json!({"input": "hi"}),
        json!({"model": 1, "input": "hi"}),
        json!("not an object"),
    ] {
        assert!(responses_view(&bad).is_err(), "{bad}");
    }
}

#[test]
fn chat_parts_keep_their_meaning() {
    let parts = json!([{"type": "text", "text": "a"}, {"type": "refusal", "refusal": "no"}]);
    assert_eq!(text_of(&parts), "a");
    let chat: slm_router::schema::ChatRequest = serde_json::from_value(json!({
        "model": "m", "messages": [{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "x"}}]}]
    }))
    .unwrap();
    assert!(features(&chat).needs_capabilities);
}
