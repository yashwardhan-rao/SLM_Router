//! A router over a three-tier `[[tier]]` ladder of fakes: small (local) -> mid -> top.

use super::{Fake, Mode, TempDir, fake, serve};
use slm_router::config::Config;

pub struct Rig3 {
    pub base: String,
    /// small, mid, top.
    pub t: [Fake; 3],
    pub dir: TempDir,
}

/// small: local, 4k context, unpriced. mid: remote, tools, $1/$2, keyed. top: remote, tools,
/// stateful, hard, $3/$15, keyed. Breakers trip after 2 failures and stay open for 30 s.
pub fn ladder_toml(urls: [&str; 3], log: &std::path::Path) -> String {
    format!(
        "[[tier]]\nname = \"small\"\nbase_url = \"{}\"\nmodel = \"small-m\"\nlocal = true\n\
         max_context_tokens = 4096\nfirst_byte_timeout_ms = 300\n\
         [[tier]]\nname = \"mid\"\nbase_url = \"{}\"\nmodel = \"mid-m\"\nsupports_tools = true\n\
         api_key_env = \"MID_KEY\"\ninput_per_mtok = 1.0\noutput_per_mtok = 2.0\nfirst_byte_timeout_ms = 300\n\
         [[tier]]\nname = \"top\"\nbase_url = \"{}\"\nmodel = \"top-m\"\nsupports_tools = true\n\
         stateful = true\nhard = true\napi_key_env = \"TOP_KEY\"\n\
         input_per_mtok = 3.0\noutput_per_mtok = 15.0\n\
         [log]\npath = {:?}\n",
        urls[0],
        urls[1],
        urls[2],
        log.to_str().unwrap()
    )
}

pub async fn rig3(modes: [Mode; 3], tweak: impl FnOnce(&mut Config)) -> Rig3 {
    let [a, b, c] = modes;
    let t = [
        fake(a, "small").await,
        fake(b, "mid").await,
        fake(c, "top").await,
    ];
    let dir = TempDir::new();
    let urls = [t[0].url.as_str(), t[1].url.as_str(), t[2].url.as_str()];
    let mut cfg = Config::from_toml(&ladder_toml(urls, &dir.log())).unwrap();
    cfg.resolve_secrets(|k| match k {
        "TOP_KEY" => Some("sk-top".into()),
        "MID_KEY" => Some("sk-mid".into()),
        _ => None,
    });
    tweak(&mut cfg);
    Rig3 {
        base: serve(cfg).await,
        t,
        dir,
    }
}
