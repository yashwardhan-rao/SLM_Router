//! Prompt token estimate by symbol density; no tokenizer dependency.
//! Checked against llama3.2's counts in `tests/tokens.rs` (logs, JSON, CSV, code, prose, CJK).

/// Letter runs cost `1 + (len-1)/6`, digit runs `ceil(len/3)` (+1 after a space, which llama3
/// splits off as its own token), each other ASCII symbol and each non-ASCII char 1. Whitespace is free.
pub fn estimate(text: &str) -> usize {
    let mut n = 0;
    let mut chars = text.chars().peekable();
    let mut prev = '\0';
    while let Some(c) = chars.next() {
        if c.is_ascii_alphabetic() || c.is_ascii_digit() {
            let digits = c.is_ascii_digit();
            let mut len: usize = 1;
            while chars
                .next_if(|d| d.is_ascii_digit() == digits && d.is_ascii_alphanumeric())
                .is_some()
            {
                len += 1;
            }
            n += match digits {
                true => len.div_ceil(3) + usize::from(prev == ' '),
                false => 1 + (len - 1) / 6,
            };
            prev = 'a';
            continue;
        }
        if !c.is_whitespace() {
            n += 1;
        }
        prev = c;
    }
    n
}
