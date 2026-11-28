use clap::Parser;
use slm_router::cli::{Cli, Command};

fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(std::iter::once("slm-router").chain(args.iter().copied()))
}

#[test]
fn serve_takes_config_bind_and_warmup() {
    let cli = parse(&[
        "serve",
        "--config",
        "a.toml",
        "--bind",
        "127.0.0.1:9",
        "--warmup",
    ])
    .unwrap();
    let Command::Serve {
        config,
        bind,
        warmup,
    } = cli.command
    else {
        panic!("expected serve")
    };
    assert_eq!(
        (config.as_str(), bind.as_deref(), warmup),
        ("a.toml", Some("127.0.0.1:9"), true)
    );
}

#[test]
fn serve_defaults_to_local_config_file_without_warmup() {
    let Command::Serve {
        config,
        bind,
        warmup,
    } = parse(&["serve"]).unwrap().command
    else {
        panic!()
    };
    assert_eq!(
        (config.as_str(), bind, warmup),
        ("slm-router.toml", None, false)
    );
}

#[test]
fn stats_takes_config_and_since() {
    let cli = parse(&["stats", "--since", "24h"]).unwrap();
    let Command::Stats { since, .. } = cli.command else {
        panic!("expected stats")
    };
    assert_eq!(since.as_deref(), Some("24h"));
}

#[test]
fn calibrate_takes_config_and_requires_labels() {
    let cli = parse(&["calibrate", "--labels", "f.jsonl", "--config", "c.toml"]).unwrap();
    let Command::Calibrate { config, labels } = cli.command else {
        panic!("expected calibrate")
    };
    assert_eq!((config.as_str(), labels.as_str()), ("c.toml", "f.jsonl"));
    assert!(parse(&["calibrate"]).is_err());
}

#[test]
fn init_defaults_to_local_config_without_force() {
    let Command::Init { path, force } = parse(&["init"]).unwrap().command else {
        panic!("expected init")
    };
    assert_eq!((path.as_str(), force), ("slm-router.toml", false));
    let Command::Init { path, force } = parse(&["init", "--path", "x.toml", "--force"])
        .unwrap()
        .command
    else {
        panic!("expected init")
    };
    assert_eq!((path.as_str(), force), ("x.toml", true));
}

#[test]
fn unknown_or_missing_subcommands_fail() {
    assert!(parse(&["explode"]).is_err());
    assert!(parse(&[]).is_err());
}
