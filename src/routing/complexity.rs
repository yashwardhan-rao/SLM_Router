//! Rule 5 scoring: reasoning cues, debugging hints, code and big-O in the last user message.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RoutingConfig {
    pub local_supports_tools: bool,
    pub local_max_context_tokens: usize,
    pub complex_prompt_tokens: usize,
    /// Cues only count in a last user message longer than this many characters.
    pub cue_min_chars: usize,
    /// Any match scores 2.
    pub reasoning_cues: Vec<String>,
    /// Each distinct match scores 1, at most 2 in total.
    pub hint_cues: Vec<String>,
    /// Score at which rule 5 sends the request to the cloud.
    pub complexity_threshold: u32,
}

const REASONING: &str = "prove, show that, derive, justify, step by step, analyze, analyse,     refactor, trade off, counterexample, induction";

/// By category: debugging, concurrency, algorithms and performance, systems, maths, question shape.
const HINTS: &str = "bug, debug, fix, compile, error, crash, hang, segfault, panic, leak, wrong,     borrow checker, race condition, deadlock, lock free, thread safe, memory order, memory ordering,     concurrency, concurrent, mutex, atomic, goroutine, thread,     algorithm, complexity, optimize, efficient, efficiently, latency, throughput, recurrence,     invariant, edge case,     design, architecture, distributed, shard, replication, consistency, consensus, cluster,     microservice, partition, rebalancing, scale, proxy, tcp,     theorem, lemma, integer, equation, summation, gradient, probability, floating point,     why, compare, implement, how would you";

fn list(csv: &str) -> Vec<String> {
    csv.split(',').map(|c| c.trim().to_string()).collect()
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            local_supports_tools: false,
            local_max_context_tokens: 4096,
            complex_prompt_tokens: 1500,
            cue_min_chars: 0,
            reasoning_cues: list(REASONING),
            hint_cues: list(HINTS),
            complexity_threshold: 2,
        }
    }
}

const CODE_MARKERS: [&str; 10] = [
    "{", ";", "=>", "->", "::", "def ", "fn ", "#include", "==", "!=",
];

/// Lowercase words split on anything that is not a letter or digit, so "lock-free" is "lock free".
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// Whole-word sequence match; the cue's last word may also take a plural "s".
fn has_cue(text: &[String], cue: &str) -> bool {
    let cue = words(cue);
    let Some((last, head)) = cue.split_last() else {
        return false;
    };
    text.windows(cue.len()).any(|w| {
        let (w_last, w_head) = w.split_last().expect("window is cue.len() >= 1 long");
        w_head == head && (w_last == last || w_last.strip_suffix('s') == Some(last.as_str()))
    })
}

fn big_o(lower: &str) -> bool {
    ["o(n", "o(log", "o(1)"].iter().any(|p| {
        lower.match_indices(p).any(|(i, _)| {
            !lower[..i]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric)
        })
    })
}

/// `a_b` (snake_case identifier) or `f(` (call): a byte pair or triple around `_` / `(`.
fn has_ident_marker(text: &str, mid: u8, needs_after: bool) -> bool {
    text.as_bytes().windows(3).any(|w| {
        w[0].is_ascii_alphanumeric()
            && w[1] == mid
            && (!needs_after || w[2].is_ascii_alphanumeric())
    })
}

fn is_code(text: &str) -> bool {
    let markers = CODE_MARKERS.iter().filter(|m| text.contains(**m)).count()
        + usize::from(has_ident_marker(text, b'_', true))
        + usize::from(has_ident_marker(text, b'(', false));
    text.contains('`') || markers >= 2
}

pub fn score(text: &str, cfg: &RoutingConfig) -> u32 {
    if text.chars().count() <= cfg.cue_min_chars {
        return 0;
    }
    let ws = words(text);
    let reasoning = cfg.reasoning_cues.iter().any(|c| has_cue(&ws, c));
    let hints = cfg.hint_cues.iter().filter(|c| has_cue(&ws, c)).count();
    2 * u32::from(reasoning)
        + hints.min(2) as u32
        + u32::from(is_code(text))
        + u32::from(big_o(&text.to_lowercase()))
}
