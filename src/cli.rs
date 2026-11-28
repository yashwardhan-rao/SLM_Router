//! Command line: `serve`, `stats`, `report`, `calibrate`, `route` and `init`, nothing else.

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "slm-router",
    version,
    about = "OpenAI-compatible router: local Ollama or cloud"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the proxy.
    Serve {
        #[arg(long, env = "ROUTER_CONFIG", default_value = "slm-router.toml")]
        config: String,
        /// Overrides `server.bind`.
        #[arg(long)]
        bind: Option<String>,
        /// Send one tiny request to the local model at startup so the first real one is not a cold load.
        #[arg(long)]
        warmup: bool,
    },
    /// Summarise the request log.
    Stats {
        #[arg(long, env = "ROUTER_CONFIG", default_value = "slm-router.toml")]
        config: String,
        /// Only include requests newer than this, e.g. `24h`.
        #[arg(long)]
        since: Option<String>,
    },
    /// Summarise the request log as Markdown, or JSON with `--json`.
    Report {
        #[arg(long, env = "ROUTER_CONFIG", default_value = "slm-router.toml")]
        config: String,
        /// Only include requests newer than this, e.g. `24h`.
        #[arg(long)]
        since: Option<String>,
        /// Print the summary as JSON instead of Markdown.
        #[arg(long)]
        json: bool,
    },
    /// Suggest a Rule 5 `classifier_threshold` from your own labelled prompts.
    Calibrate {
        #[arg(long, env = "ROUTER_CONFIG", default_value = "slm-router.toml")]
        config: String,
        /// JSONL, one `{"prompt": "...", "label": "local" | "cloud"}` per line.
        #[arg(long)]
        labels: String,
        /// Instead of best F1: catch the most cloud prompts while keeping at least this share
        /// of local prompts local, e.g. 0.95.
        #[arg(long, value_parser = crate::calibrate::fraction, conflicts_with = "max_false_local")]
        min_local_recall: Option<f64>,
        /// Instead of best F1: keep the most local prompts local while wrongly keeping at most
        /// this share of cloud prompts local, e.g. 0.05.
        #[arg(long, value_parser = crate::calibrate::fraction)]
        max_false_local: Option<f64>,
    },
    /// Print the routing decision and its time for each prompt, offline; no upstream is called.
    /// Hidden: a benchmarking aid for `scripts/eval.py --bench`, outside the stable CLI.
    #[command(hide = true)]
    Route {
        #[arg(long, env = "ROUTER_CONFIG", default_value = "slm-router.toml")]
        config: String,
        /// JSONL, one `{"prompt": "..."}` per line; other fields are ignored.
        #[arg(long)]
        prompts: String,
    },
    /// Write the documented example config to start from.
    Init {
        #[arg(long, default_value = "slm-router.toml")]
        path: String,
        /// Overwrite an existing file.
        #[arg(long)]
        force: bool,
    },
}
