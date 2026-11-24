//! Command line: `serve` and `stats`, nothing else.

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
}
