//! `ccdl` command-line interface.
#![deny(missing_docs)]

mod output;

use std::io::Write;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use futures::StreamExt;

use ccdl::Ccdl;
use ccdl::error::Error;
use ccdl::model::crawl::CrawlSelector;
use ccdl::model::query::{Collapse, MatchType, UrlQuery};

use output::{Format, Writer};

/// Common Crawl download & enumeration tool.
#[derive(Debug, Parser)]
#[command(name = "ccdl", version, about)]
struct Cli {
    /// Cache directory (defaults to XDG cache dir).
    #[arg(long, global = true)]
    cache_dir: Option<String>,

    /// Maximum requests per second per host.
    #[arg(long, global = true, default_value_t = 2)]
    max_rps: u32,

    /// Crawl selector (e.g. `latest-4`, `2020..2024`, `all`).
    #[arg(long, global = true)]
    crawls: Option<String>,

    /// Output format: ndjson|csv|table.
    #[arg(short, long, global = true, default_value = "ndjson")]
    output: String,

    /// User-Agent string for requests.
    #[arg(
        long,
        global = true,
        default_value = "ccdl/0.0.1 (+https://github.com/akiselev/ccdl)"
    )]
    user_agent: String,

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
    Stats {
        /// URL pattern.
        pattern: String,
        /// Match type: exact|prefix|host|domain.
        #[arg(long, default_value = "prefix")]
        r#match: String,
    },
    /// Search the index for captures.
    Search {
        /// URL pattern.
        pattern: String,
        /// Match type: exact|prefix|host|domain.
        #[arg(long, default_value = "prefix")]
        r#match: String,
        /// Filter to an exact status.
        #[arg(long)]
        status: Option<u16>,
        /// Filter to a MIME type.
        #[arg(long)]
        mime: Option<String>,
        /// Collapse directive: digest|urlkey.
        #[arg(long)]
        collapse: Option<String>,
        /// Print the compiled CDX URL(s) instead of running.
        #[arg(long)]
        dry_run: bool,
    },
    /// Enumerate captures for a URL pattern.
    Enumerate {
        /// URL pattern.
        pattern: String,
        /// Match type: exact|prefix|host|domain.
        #[arg(long, default_value = "prefix")]
        r#match: String,
        /// Filter to an exact status.
        #[arg(long)]
        status: Option<u16>,
        /// Deduplicate by url key.
        #[arg(long)]
        distinct: bool,
        /// Print the compiled query instead of running.
        #[arg(long)]
        dry_run: bool,
    },
}

fn parse_match(s: &str) -> MatchType {
    match s.to_ascii_lowercase().as_str() {
        "exact" => MatchType::Exact,
        "host" => MatchType::Host,
        "domain" => MatchType::Domain,
        _ => MatchType::Prefix,
    }
}

fn exit_for(err: &Error) -> ExitCode {
    match err {
        Error::RateLimited { .. } => ExitCode::from(3),
        Error::IndexTooLarge { .. } => ExitCode::from(4),
        Error::Config(_) => ExitCode::from(2),
        _ => ExitCode::FAILURE,
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ccdl: {e}");
            exit_for(&e)
        }
    }
}

async fn run(cli: Cli) -> Result<(), Error> {
    let selector: CrawlSelector = match &cli.crawls {
        Some(s) => s.parse()?,
        None => CrawlSelector::Latest,
    };
    let format = Format::parse(&cli.output);

    let mut builder = Ccdl::builder()
        .max_rps(cli.max_rps)
        .user_agent(cli.user_agent.clone());
    if let Some(dir) = &cli.cache_dir {
        builder = builder.cache_dir(dir);
    }

    match cli.command {
        Command::Crawls { refresh } => {
            let client = builder.build()?;
            if refresh {
                client.registry().refresh().await?;
            }
            let stdout = std::io::stdout();
            let mut out = stdout.lock();
            for c in client.crawls().await? {
                let _ = writeln!(out, "{}\t{}", c.id, c.name);
            }
            Ok(())
        }
        Command::Stats { pattern, r#match } => {
            let client = builder.build()?;
            let q = UrlQuery {
                pattern,
                match_type: parse_match(&r#match),
                crawls: selector,
                filters: vec![],
                collapse: None,
                fields: None,
                time_range: None,
                limit: None,
            };
            let est = client.stats(&q).await?;
            println!("{{\"pages\":{}}}", est.pages);
            Ok(())
        }
        Command::Search {
            pattern,
            r#match,
            status,
            mime,
            collapse,
            dry_run,
        } => {
            let mut q = UrlQuery {
                pattern,
                match_type: parse_match(&r#match),
                crawls: selector,
                filters: vec![],
                collapse: None,
                fields: None,
                time_range: None,
                limit: None,
            };
            if let Some(s) = status {
                q = q.status(s);
            }
            if let Some(m) = mime {
                q = q.mime(m);
            }
            if let Some(c) = collapse {
                q.collapse = match c.as_str() {
                    "urlkey" => Some(Collapse::UrlKey),
                    _ => Some(Collapse::Digest),
                };
            }
            if dry_run {
                let client = builder.build()?;
                for crawl in client.registry().resolve(&q.crawls).await? {
                    println!("{}", ccdl::index::compile_cdx_url(&crawl, &q, &[]));
                }
                return Ok(());
            }
            let client = builder.build()?;
            let stream = client.search(q).await?;
            write_stream(stream, format).await
        }
        Command::Enumerate {
            pattern,
            r#match,
            status,
            distinct,
            dry_run,
        } => {
            let client = builder.build()?;
            let mut b = client
                .enumerate(pattern, parse_match(&r#match))
                .crawls(selector);
            if let Some(s) = status {
                b = b.status(s);
            }
            if distinct {
                b = b.distinct_urls();
            }
            if dry_run {
                for crawl in client.registry().resolve(&b.query().crawls).await? {
                    println!("{}", ccdl::index::compile_cdx_url(&crawl, b.query(), &[]));
                }
                return Ok(());
            }
            let stream = b.run().await?;
            write_stream(stream, format).await
        }
    }
}

async fn write_stream(mut stream: ccdl::index::CaptureStream, format: Format) -> Result<(), Error> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut writer = Writer::new(format);
    while let Some(item) = stream.next().await {
        let cap = item?;
        let _ = writer.write(&mut out, &cap);
    }
    Ok(())
}
