//! Hashed n-gram features for the Rule 5 classifier. `scripts/features.py` is the Python twin the
//! trainer uses; `tests/fixtures/featurizer_parity.json` pins the two together.

use super::complexity::{big_o, is_code, words};

pub const BUCKETS: usize = 1 << 16;

/// FNV-1a 32 over the concatenation of `parts`.
pub fn fnv1a(parts: &[&[u8]]) -> u32 {
    parts
        .iter()
        .flat_map(|p| p.iter())
        .fold(0x811c_9dc5, |h, &b| {
            (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
        })
}

/// The set of active buckets: word unigrams, word bigrams, and char 3..=5-grams of `" word "`.
pub struct Sparse {
    bits: [u64; BUCKETS / 64],
}

impl Sparse {
    pub fn of(text: &str) -> Self {
        let mut s = Sparse {
            bits: [0; BUCKETS / 64],
        };
        let ws = words(text);
        let (mut padded, mut ends) = (String::new(), Vec::new());
        for (i, w) in ws.iter().enumerate() {
            s.set(&[b"w:", w.as_bytes()]);
            if let Some(next) = ws.get(i + 1) {
                s.set(&[b"b:", w.as_bytes(), b" ", next.as_bytes()]);
            }
            padded.clear();
            padded.extend([" ", w, " "]);
            ends.clear();
            ends.extend(padded.char_indices().map(|(i, _)| i));
            ends.push(padded.len());
            for n in 3..=5 {
                for win in ends.windows(n + 1) {
                    s.set(&[b"c:", &padded.as_bytes()[win[0]..win[n]]]);
                }
            }
        }
        s
    }

    fn set(&mut self, parts: &[&[u8]]) {
        let b = fnv1a(parts) as usize % BUCKETS;
        self.bits[b / 64] |= 1 << (b % 64);
    }

    /// Active buckets in ascending order.
    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.bits.iter().enumerate().flat_map(|(i, &word)| {
            let mut rest = word;
            std::iter::from_fn(move || {
                let b = rest.trailing_zeros();
                (rest != 0).then(|| {
                    rest &= rest - 1;
                    (i * 64) as u32 + b
                })
            })
        })
    }

    /// The sparse dot product is divided by this, so long prompts don't score higher by length.
    pub fn norm(&self) -> f32 {
        let active: u32 = self.bits.iter().map(|w| w.count_ones()).sum();
        (active as f32).sqrt()
    }
}

/// `[is_code, big_o, cue_score / 4, log2(tokens) / 12]`, each clamped to `[0, 1]`.
pub fn dense(text: &str, est_tokens: usize, cue_score: u32) -> [f32; 4] {
    let tokens = (est_tokens.max(1) as f32).log2().min(12.0) / 12.0;
    [
        f32::from(u8::from(is_code(text))),
        f32::from(u8::from(big_o(&text.to_lowercase()))),
        cue_score.min(4) as f32 / 4.0,
        tokens,
    ]
}
