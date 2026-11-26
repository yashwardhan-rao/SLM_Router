use slm_router::config::Config;
use slm_router::routing::Policy;

const MINIMAL: &str = r#"
[local]
base_url = "http://127.0.0.1:11434/v1"
model = "llama3.2"
[cloud]
base_url = "https://api.example.com/v1"
model = "big-model"
api_key_env = "CLOUD_KEY"
"#;

#[test]
fn minimal_config_gets_spec_defaults() {
    let c = Config::from_toml(MINIMAL).unwrap();
    assert_eq!(c.server.bind, "127.0.0.1:8080");
    assert_eq!(c.server.policy, Policy::Auto);
    assert_eq!(c.local.first_byte_timeout_ms, 15000);
    assert_eq!(c.session.ttl_secs, 300);
    assert_eq!(c.session.max_pins, 10_000);
    assert_eq!(c.routing.complex_prompt_tokens, 1500);
    assert_eq!(c.routing.local_max_context_tokens, 4096);
    assert!(c.pricing.is_none());
    assert!(c.validate().is_ok());
}

#[test]
fn policy_and_routing_overrides_are_read() {
    let t = format!(
        "[server]\npolicy = \"local-only\"\n[routing]\ncomplex_prompt_tokens = 10\n{MINIMAL}"
    );
    let c = Config::from_toml(&t).unwrap();
    assert_eq!(c.server.policy, Policy::LocalOnly);
    assert_eq!(c.routing.complex_prompt_tokens, 10);
}

#[test]
fn unknown_fields_and_missing_sections_are_rejected() {
    let typo = format!("{MINIMAL}\n[session]\nttl_sec = 5\n");
    assert!(Config::from_toml(&typo).is_err());
    assert!(Config::from_toml("[local]\nbase_url = \"x\"\nmodel = \"m\"").is_err());
    assert!(Config::from_toml("not toml ===").is_err());
}

#[test]
fn non_loopback_bind_requires_router_api_key() {
    let t = format!("[server]\nbind = \"0.0.0.0:8080\"\n{MINIMAL}");
    let mut c = Config::from_toml(&t).unwrap();
    assert!(c.validate().is_err());
    c.server.api_key = Some("secret".into());
    assert!(c.validate().is_ok());
}

#[test]
fn secrets_come_from_the_environment_never_the_file() {
    let mut c = Config::from_toml(MINIMAL).unwrap();
    c.resolve_secrets(|name| match name {
        "CLOUD_KEY" => Some("sk-cloud".into()),
        "ROUTER_API_KEY" => Some("router".into()),
        _ => None,
    });
    assert_eq!(c.cloud.api_key.as_deref(), Some("sk-cloud"));
    assert_eq!(c.server.api_key.as_deref(), Some("router"));
}

#[test]
fn max_body_bytes_defaults_to_64_mib_and_is_configurable() {
    assert_eq!(
        Config::from_toml(MINIMAL).unwrap().server.max_body_bytes,
        67_108_864
    );
    let c = Config::from_toml(&format!("[server]\nmax_body_bytes = 1000\n{MINIMAL}")).unwrap();
    assert_eq!(c.server.max_body_bytes, 1000);
}

#[test]
fn empty_secret_env_vars_count_as_unset() {
    let mut c = Config::from_toml(MINIMAL).unwrap();
    c.resolve_secrets(|_| Some(String::new()));
    assert_eq!((c.cloud.api_key, c.server.api_key), (None, None));
}

#[test]
fn classifier_threshold_defaults_to_a_probability_and_is_validated() {
    let mut c = Config::from_toml(MINIMAL).unwrap();
    let t = c.routing.classifier_threshold;
    assert!(t > 0.0 && t <= 1.0, "calibrated default {t}");
    let off = format!("[routing]\nclassifier_threshold = 1.5\n{MINIMAL}");
    assert_eq!(
        Config::from_toml(&off)
            .unwrap()
            .routing
            .classifier_threshold,
        1.5
    );
    for bad in [0.0, -0.5, f32::NAN] {
        c.routing.classifier_threshold = bad;
        assert!(c.validate().is_err(), "{bad} must be rejected");
    }
}

#[test]
fn the_documented_example_config_parses_and_validates() {
    let c = Config::from_toml(include_str!("../slm-router.example.toml")).unwrap();
    assert!(c.validate().is_ok());
}
