//! `[[tier]]` config: parsing, defaults, and the shapes it rejects.

use slm_router::config::Config;
use std::time::Duration;

const TWO: &str = r#"
[[tier]]
name = "small"
base_url = "http://127.0.0.1:11434/v1"
model = "llama3.2"
local = true
[[tier]]
name = "big"
base_url = "https://api.example.com/v1"
model = "big-model"
api_key_env = "BIG_KEY"
supports_tools = true
stateful = true
hard = true
input_per_mtok = 2.5
output_per_mtok = 10.0
"#;

fn err(toml: &str) -> String {
    Config::from_toml(toml).unwrap_err()
}

#[test]
fn a_tier_ladder_parses_with_defaults() {
    let mut c = Config::from_toml(TWO).unwrap();
    assert!(c.validate().is_ok());
    c.resolve_secrets(|k| (k == "BIG_KEY").then(|| "sk-big".to_string()));
    let l = c.ladder();
    assert_eq!(
        l.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["small", "big"]
    );
    assert_eq!(l[1].api_key.as_deref(), Some("sk-big"));
    assert_eq!(
        (l[0].breaker_failures, l[0].breaker_cooldown_ms),
        (2, 30_000)
    );
    assert_eq!(
        l[0].first_byte_timeout(),
        Some(Duration::from_millis(15_000))
    );
    assert_eq!(l[1].first_byte_timeout(), None);
    assert!(l[0].pricing().is_none() && l[1].pricing().is_some());
    assert_eq!(c.routing.tiers.len(), 2);
}

#[test]
fn the_legacy_form_becomes_a_two_tier_ladder() {
    let legacy = "[local]\nbase_url = \"l\"\nmodel = \"m\"\n\
                  [cloud]\nbase_url = \"c\"\nmodel = \"n\"\napi_key_env = \"K\"\n";
    let l = Config::from_toml(legacy).unwrap().ladder();
    assert_eq!((l[0].name.as_str(), l[0].local), ("local", true));
    assert_eq!(
        (l[1].name.as_str(), l[1].stateful, l[1].hard),
        ("cloud", true, true)
    );
    assert_eq!(
        l[1].breaker_failures, 0,
        "the legacy cloud never had a breaker"
    );
}

#[test]
fn mixed_empty_or_ambiguous_ladders_are_rejected() {
    let legacy = "[local]\nbase_url = \"l\"\nmodel = \"m\"\n";
    assert!(err(&format!("{TWO}{legacy}")).contains("not both"));
    assert!(err("[log]\npath = \"x\"\n").contains("needs [[tier]]"));
    let dup = TWO.replace("name = \"big\"", "name = \"small\"");
    assert!(err(&dup).contains("unique"));
    let auto = TWO.replace("name = \"big\"", "name = \"auto\"");
    assert!(err(&auto).contains("auto"));
    let pricing = format!("{TWO}[pricing]\ninput_per_mtok = 1.0\noutput_per_mtok = 1.0\n");
    assert!(err(&pricing).contains("[pricing]"));
}

#[test]
fn prices_must_be_non_negative_and_set_in_pairs() {
    let neg = TWO.replace("input_per_mtok = 2.5", "input_per_mtok = -1.0");
    assert!(err(&neg).contains(">= 0"));
    let half = TWO.replace("output_per_mtok = 10.0\n", "");
    assert!(err(&half).contains("both"));
    let legacy_neg = "[local]\nbase_url = \"l\"\nmodel = \"m\"\n\
                      [cloud]\nbase_url = \"c\"\nmodel = \"n\"\napi_key_env = \"K\"\n\
                      [pricing]\ninput_per_mtok = 1.0\noutput_per_mtok = -2.0\n";
    assert!(err(legacy_neg).contains(">= 0"));
}

#[test]
fn ignored_routing_keys_and_a_misordered_ladder_warn() {
    assert!(Config::from_toml(TWO).unwrap().warnings().is_empty());
    let ignored = format!("[routing]\nlocal_supports_tools = true\n{TWO}");
    let w = Config::from_toml(&ignored).unwrap().warnings();
    assert!(w.len() == 1 && w[0].contains("ignored"), "{w:?}");
    let pricey = TWO.replacen(
        "local = true",
        "local = true\ninput_per_mtok = 9.0\noutput_per_mtok = 0.0",
        1,
    );
    let w = Config::from_toml(&pricey).unwrap().warnings();
    assert!(w.len() == 1 && w[0].contains("\"small\""), "{w:?}");
}
