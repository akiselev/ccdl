//! `ccdl` command-line interface.
#![deny(missing_docs)]

use clap::{Parser, Subcommand};

/// Common Crawl download & enumeration tool.
#[derive(Debug, Parser)]
#[command(name = "ccdl", version, about)]
struct Cli {
    /// Cache directory (defaults to XDG cache dir).
    #[arg(long, global = true)]
    cache_dir: Option<String>,

    /// Maximum requests per second per host.
    #[arg(long, global = true)]
    max_rps: Option<f64>,

    /// Crawl selector (e.g. `latest-4`, `2020..2024`, `all`).
    #[arg(long, global = true)]
    crawls: Option<String>,

    /// Output format.
    #[arg(short, long, global = true, default_value = "ndjson")]
    output: String,

    /// User-Agent string for requests.
    #[arg(long, global = true)]
    user_agent: Option<String>,

    /// Increase verbosity.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List available crawls.
    Crawls {
        /// Refresh the cached crawl registry.
        #[arg(long)]
        refresh: bool,
    },
    /// Estimate the size of a query.
    Stats { pattern: String },
    /// Search the index for captures.
    Search { pattern: String },
    /// Enumerate captures for a URL pattern.
    Enumerate { pattern: String },
}

fn main() {
    let cli = Cli::parse();
    let _ = cli;
    eprintln!("ccdl: not yet implemented");
}
