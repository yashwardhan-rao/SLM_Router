use super::SseScan;

fn scan(chunks: &[&str]) -> SseScan {
    let mut s = SseScan::default();
    chunks.iter().for_each(|c| {
        s.feed(c.as_bytes());
    });
    s
}

const CREATED: &str = "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"usage\":null}}\n\n";
const DELTA: &str = "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"12345678\"}\n\n";

fn terminal(kind: &str) -> String {
    format!(
        "event: response.{kind}\ndata: {{\"type\":\"response.{kind}\",\"response\":{{\"usage\":{{\"input_tokens\":27,\"output_tokens\":3,\"total_tokens\":30}}}}}}\n\n"
    )
}

#[test]
fn responses_usage_comes_from_every_terminal_event_split_or_not() {
    for kind in ["completed", "incomplete", "failed"] {
        let t = terminal(kind);
        let (a, b) = t.split_at(t.len() / 2);
        assert_eq!(
            scan(&[CREATED, DELTA, a, b]).tokens(99),
            (27, 3, false),
            "{kind}"
        );
    }
}

#[test]
fn done_sentinel_is_optional() {
    let t = terminal("completed");
    assert_eq!(scan(&[DELTA, &t]).tokens(99), (27, 3, false));
    assert_eq!(
        scan(&[DELTA, &t, "data: [DONE]\n\n"]).tokens(99),
        (27, 3, false)
    );
}

#[test]
fn a_null_usage_after_a_real_one_does_not_clobber_it() {
    assert_eq!(
        scan(&[&terminal("completed"), CREATED]).tokens(99),
        (27, 3, false)
    );
}

#[test]
fn deltas_without_a_terminal_event_are_estimated() {
    assert_eq!(scan(&[CREATED, DELTA, DELTA]).tokens(99), (99, 4, true));
}

#[test]
fn chat_usage_still_parses() {
    let chunks = [
        "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n\n",
        "data: [DONE]\n\n",
    ];
    assert_eq!(scan(&chunks).tokens(99), (7, 3, false));
}

const CHAT_USAGE: &str =
    "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n\n";
const CHAT_DELTA: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n";

fn filtered(chunks: &[&[u8]]) -> (Vec<u8>, SseScan) {
    let mut s = SseScan::hiding_usage();
    let mut out: Vec<u8> = chunks.iter().flat_map(|c| s.feed(c)).collect();
    out.extend(s.tail());
    (out, s)
}

#[test]
fn filter_drops_only_the_usage_chunk_and_still_counts_it() {
    let done = "data: [DONE]\n\n";
    let (out, s) = filtered(&[
        CHAT_DELTA.as_bytes(),
        CHAT_USAGE.as_bytes(),
        done.as_bytes(),
    ]);
    assert_eq!(
        String::from_utf8(out).unwrap(),
        format!("{CHAT_DELTA}{done}")
    );
    assert_eq!(s.tokens(99), (7, 3, false));
}

#[test]
fn filter_never_loses_an_unterminated_last_line() {
    let (out, _) = filtered(&[CHAT_DELTA.as_bytes(), b"data: {\"partial"]);
    assert_eq!(out, format!("{CHAT_DELTA}data: {{\"partial").into_bytes());
}

#[test]
fn filter_forwards_an_oversized_line_instead_of_dropping_it() {
    let big = vec![b'x'; (1 << 20) + 1];
    let (out, _) = filtered(&[&big]);
    assert_eq!(out, big);
}

#[test]
fn a_plain_scan_forwards_nothing_itself() {
    let mut s = SseScan::default();
    assert!(s.feed(CHAT_USAGE.as_bytes()).is_empty());
    assert!(s.tail().is_empty());
}
