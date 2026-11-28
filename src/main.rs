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
            match read_log(&config, since.as_deref())? {
                Some(s) => println!("{}", stats::text(&s)),
                None => println!("no requests logged yet"),
            }
            Ok(())
        }
        Command::Report {
            config,
            since,
            json,
        } => {
            let s = read_log(&config, since.as_deref())?
                .unwrap_or_else(|| stats::summarize(&b""[..], 0, None));
            match json {
                true => println!("{}", serde_json::to_string_pretty(&s).expect("plain data")),
                false => print!("{}", stats::markdown(&s, since.as_deref())),
            }
            Ok(())
        }
        Command::Calibrate {
            config,
            labels,
            min_local_recall,
            max_false_local,
        } => {
            let cfg = offline_config(&config)?;
            let text = std::fs::read_to_string(&labels)
                .map_err(|e| format!("cannot read {labels}: {e}"))?;
            let target = match (min_local_recall, max_false_local) {
                (Some(r), _) => calibrate::Target::MinLocalRecall(r),
                (_, Some(f)) => calibrate::Target::MaxFalseLocal(f),
                _ => calibrate::Target::F1,
            };
            let out = calibrate::threshold(&calibrate::parse(&text)?, &cfg.routing, target)?;
            println!("{}", calibrate::render(&out));
            Ok(())
        }
        Command::Route { config, prompts } => {
            let text = std::fs::read_to_string(&prompts)
                .map_err(|e| format!("cannot read {prompts}: {e}"))?;
            println!(
                "{}",
                slm_router::route::run(offline_config(&config)?, &text)?
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

/// The log summarised over the `--since` window; None when nothing has been logged yet.
fn read_log(config: &str, since: Option<&str>) -> Result<Option<stats::Summary>, String> {
    let since = since.map(stats::parse_duration).transpose()?;
    let cfg = offline_config(config)?;
    match std::fs::File::open(&cfg.log.path) {
        Ok(f) => {
            let log = std::io::BufReader::new(f);
            Ok(Some(stats::summarize(log, logging::now_ms(), since)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read {}: {e}", cfg.log.path)),
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
