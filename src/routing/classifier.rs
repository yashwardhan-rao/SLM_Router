//! Rule 5 classifier: L2 logistic regression over `features`, trained by `scripts/train.py` and
//! embedded at build time, so scoring a prompt does no I/O.
//!
//! `assets/rule5.bin`, little-endian: magic `SLR5`, u16 version (1), u32 bucket count, f32 sparse
//! scale, f32 bias, f32 calibrated threshold, 4 x f32 dense weights, then one i8 weight per bucket.

use std::sync::OnceLock;

use super::features::{BUCKETS, Sparse, dense};

const HEADER: usize = 4 + 2 + 4 + 4 * 3 + 4 * 4;

#[derive(Debug)]
pub struct Model {
    scale: f32,
    bias: f32,
    pub threshold: f32,
    dense: [f32; 4],
    sparse: &'static [u8],
}

pub fn decode(bytes: &'static [u8]) -> Result<Model, String> {
    if bytes.get(..4) != Some(b"SLR5".as_slice()) {
        return Err("bad magic: not a Rule 5 model".into());
    }
    if bytes.len() != HEADER + BUCKETS {
        return Err(format!(
            "{} bytes, expected {}",
            bytes.len(),
            HEADER + BUCKETS
        ));
    }
    let u = |at: usize, n: usize| -> u32 {
        bytes[at..at + n]
            .iter()
            .rev()
            .fold(0, |acc, &b| acc << 8 | u32::from(b))
    };
    let f = |at: usize| f32::from_bits(u(at, 4));
    if u(4, 2) != 1 || u(6, 4) as usize != BUCKETS {
        return Err(format!(
            "unsupported version {} or buckets {}",
            u(4, 2),
            u(6, 4)
        ));
    }
    Ok(Model {
        scale: f(10),
        bias: f(14),
        threshold: f(18),
        dense: [f(22), f(26), f(30), f(34)],
        sparse: &bytes[HEADER..],
    })
}

/// The embedded model, decoded once; a corrupt asset panics on first use, i.e. at startup.
pub fn model() -> &'static Model {
    static MODEL: OnceLock<Model> = OnceLock::new();
    MODEL
        .get_or_init(|| decode(include_bytes!("../../assets/rule5.bin")).expect("assets/rule5.bin"))
}

impl Model {
    pub fn prob(&self, text: &str, est_tokens: usize, cue_score: u32) -> f32 {
        let s = Sparse::of(text);
        let (sum, n) = s.iter().fold((0i32, 0u32), |(sum, n), b| {
            (sum + i32::from(self.sparse[b as usize] as i8), n + 1)
        });
        let sparse = if n == 0 {
            0.0
        } else {
            sum as f32 * self.scale / s.norm()
        };
        let d = dense(text, est_tokens, cue_score);
        let z = self.bias + sparse + d.iter().zip(self.dense).map(|(v, w)| v * w).sum::<f32>();
        1.0 / (1.0 + (-z).exp())
    }
}

/// Probability that the local model's answer to `text` would not be good enough.
pub fn prob(text: &str, est_tokens: usize, cue_score: u32) -> f32 {
    model().prob(text, est_tokens, cue_score)
}
