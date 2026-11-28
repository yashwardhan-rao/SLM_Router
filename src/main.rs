use clap::Parser;
use serde_json::json;
use slm_router::{
    app, calibrate,
    cli::{Cli, Command},
    config::Config,
    logging, stats,
    upstream::{Api, post_json},
};
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("slm-router: {e}");
            ExitCode::from(2)
        }
    }
}

async fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Serve {
            config,
            bind,
            warmup,
        } => {
            let cfg = Config::load(&config, bind)?;
            for w in cfg.warnings() {
                eprintln!("slm-router: warning: {w}");
            }
            if warmup {
                warm_up(&cfg).await;
            }
            let listener = tokio::net::TcpListener::bind(&cfg.server.bind)
                .await
                .map_err(|e| format!("cannot bind {}: {e}", cfg.server.bind))?;
            eprintln!("slm-router: listening on http://{}", cfg.server.bind);
            axum::serve(listener, app::router(cfg))
                .await
                .map_err(|e| e.to_string())
        }
        Command::Stats { config, since } => {
            let since = since.as_deref().map(stats::parse_duration).transpose()?;
            let cfg = offline_config(&config)?;
            match std::fs::File::open(&cfg.log.path) {
                Ok(f) => {
                    let log = std::io::BufReader::new(f);
                    println!("{}", stats::report(log, logging::now_ms(), since))
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    println!("no requests logged yet")
                }
                Err(e) => return Err(format!("cannot read {}: {e}", cfg.log.path)),
            }
            Ok(())
        }
        Command::Calibrate { config, labels } => {
            let cfg = offline_config(&config)?;
            let text = std::fs::read_to_string(&labels)
                .map_err(|e| format!("cannot read {labels}: {e}"))?;
            let rows = calibrate::parse(&text)?;
            let out = calibrate::threshold(&rows, &cfg.routing)?;
            println!(
                "{}",
                calibrate::render(&out, cfg.routing.classifier_threshold)
            );
            Ok(())
        }
        Command::Init { path, force } => {
            slm_router::init::write(std::path::Path::new(&path), force)?;
            println!(
                "wrote {path}: set your models and api_key_env, then run `slm-router serve --config {path}`"
            );
            Ok(())
        }
    }
}

/// Config for the offline commands: serve-only checks (bind, ROUTER_API_KEY) do not apply.
fn offline_config(path: &str) -> Result<Config, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    Config::from_toml(&text)
}

/// Best effort: a failed warm-up only means the first real request pays the cold load.
/// Local tiers warm in parallel, each under its own first-byte timeout, so one slow tier
/// never holds up the others.
async fn warm_up(cfg: &Config) {
    let http = reqwest::Client::new();
    let mut jobs = tokio::task::JoinSet::new();
    for t in cfg.ladder().into_iter().filter(|t| t.local) {
        let http = http.clone();
        jobs.spawn(async move {
            let body = json!({"model": t.model, "max_tokens": 1,
                "messages": [{"role": "user", "content": "hi"}]});
            let wait = t.first_byte_timeout();
            let r = post_json(
                &http,
                &t.base_url,
                Api::Chat,
                t.api_key.as_deref(),
                &body,
                wait,
            )
            .await;
            match r {
                Ok(reply) => eprintln!(
                    "slm-router: warm-up of {} answered with HTTP {}",
                    t.name, reply.status
                ),
                Err(e) => eprintln!("slm-router: warm-up of {} failed: {}", t.name, e.0),
            }
        });
    }
    jobs.join_all().await;
}
