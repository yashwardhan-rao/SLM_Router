mod common;

use common::*;
use slm_router::config::Config;
use slm_router::init::{EXAMPLE, write};

#[test]
fn init_writes_a_config_that_loads_and_validates() {
    let dir = TempDir::new();
    let path = dir.0.join("slm-router.toml");
    write(&path, false).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text, EXAMPLE);
    Config::from_toml(&text).unwrap().validate().unwrap();
    // No temp file is left behind.
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[test]
fn init_refuses_to_overwrite_without_force() {
    let dir = TempDir::new();
    let path = dir.0.join("slm-router.toml");
    std::fs::write(&path, b"mine").unwrap();
    let err = write(&path, false).unwrap_err();
    assert!(err.contains("--force"), "{err}");
    assert_eq!(std::fs::read(&path).unwrap(), b"mine");
}

#[test]
fn init_force_overwrites_even_a_read_only_file() {
    let dir = TempDir::new();
    let path = dir.0.join("slm-router.toml");
    std::fs::write(&path, b"mine").unwrap();
    write(&path, true).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), EXAMPLE);

    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&path, perms).unwrap();
    write(&path, true).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), EXAMPLE);
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
}

#[test]
fn init_rejects_a_missing_directory_or_a_directory_target() {
    let dir = TempDir::new();
    let err = write(&dir.0.join("nope").join("c.toml"), false).unwrap_err();
    assert!(err.contains("does not exist"), "{err}");
    assert!(!dir.0.join("nope").exists());
    let err = write(&dir.0, true).unwrap_err();
    assert!(err.contains("directory"), "{err}");
}

#[tokio::test]
async fn the_generated_config_routes_against_fake_upstreams() {
    let dir = TempDir::new();
    let path = dir.0.join("slm-router.toml");
    write(&path, false).unwrap();
    let mut cfg = Config::from_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let local = fake(Mode::Ok, "local").await;
    let cloud = fake(Mode::Ok, "cloud").await;
    cfg.local.as_mut().unwrap().base_url = local.url.clone();
    let c = cfg.cloud.as_mut().unwrap();
    c.base_url = cloud.url.clone();
    c.api_key = Some("sk-test".into());
    cfg.log.path = dir.log().to_str().unwrap().into();
    let base = serve(cfg).await;

    let (status, body) = post_chat(&base, &user("What is the capital of France?"), &[]).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!((local.count(), cloud.count()), (1, 0));
    let (status, _) = post_chat(
        &base,
        &serde_json::json!({"model": "slm-router/cloud",
            "messages": [{"role": "user", "content": "hi"}]}),
        &[],
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(cloud.count(), 1);
    assert_eq!(cloud.last().body["model"], "gpt-6.2-sol");
}
