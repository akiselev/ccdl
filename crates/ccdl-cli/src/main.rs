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
    /// Fetch a URL's captured content (or `@manifest.ndjson` of captures).
    Fetch {
        /// A URL, or `@file.ndjson` / `@-` to stream captures from NDJSON.
        target: String,
        /// Write payloads into this directory (digest-named files).
        #[arg(long)]
        out: Option<String>,
        /// Print decoded text to stdout instead of writing files.
        #[arg(long)]
        text: bool,
    },
    /// Resolve the newest 200 capture of a URL and print its decoded text.
    Text {
        /// The URL to fetch.
        url: String,
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
    let client = builder.build()?;

    match cli.command {
        Command::Crawls { refresh } => cmd_crawls(&client, refresh).await,
        Command::Stats { pattern, r#match } => {
            let q = base_query(pattern, &r#match, selector);
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
            let mut q = base_query(pattern, &r#match, selector);
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
                return dump_urls(&client, &q).await;
            }
            let stream = client.search(q).await?;
            write_stream(stream, format).await
        }
        Command::Text { url } => {
            let text = client.text(&url).await?;
            println!("{text}");
            Ok(())
        }
        Command::Fetch { target, out, text } => {
            cmd_fetch(&client, &target, out.as_deref(), text).await
        }
        Command::Enumerate {
            pattern,
            r#match,
            status,
            distinct,
            dry_run,
        } => {
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
                return dump_urls(&client, b.query()).await;
            }
            let stream = b.run().await?;
            write_stream(stream, format).await
        }
    }
}

fn base_query(pattern: String, match_str: &str, crawls: CrawlSelector) -> UrlQuery {
    UrlQuery {
        pattern,
        match_type: parse_match(match_str),
        crawls,
        filters: vec![],
        collapse: None,
        fields: None,
        time_range: None,
        limit: None,
    }
}

async fn cmd_crawls(client: &Ccdl, refresh: bool) -> Result<(), Error> {
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

async fn cmd_fetch(
    client: &Ccdl,
    target: &str,
    out: Option<&str>,
    text: bool,
) -> Result<(), Error> {
    if let Some(path) = target.strip_prefix('@') {
        let content = if path == "-" {
            let mut buf = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
            buf
        } else {
            std::fs::read_to_string(path)?
        };
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let cap: ccdl::model::capture::Capture =
                serde_json::from_str(line).map_err(|e| Error::Parse(e.to_string()))?;
            fetch_one(client, &cap, out, text).await?;
        }
        Ok(())
    } else {
        // A bare URL: resolve newest 200 capture across all crawls.
        let http = client.fetch_url(target, CrawlSelector::All).await?;
        if text {
            println!("{}", http.text()?);
        } else if let Some(dir) = out {
            write_payload(dir, target, &http.decoded_body()?)?;
        } else {
            print!("{}", http.text()?);
        }
        Ok(())
    }
}

async fn fetch_one(
    client: &Ccdl,
    cap: &ccdl::model::capture::Capture,
    out: Option<&str>,
    text: bool,
) -> Result<(), Error> {
    let http = client.fetch_http(cap).await?;
    if text {
        println!("{}", http.text()?);
    } else if let Some(dir) = out {
        let name = cap.digest.replace(':', "_");
        write_payload(dir, &name, &http.decoded_body()?)?;
    } else {
        let _ = std::io::Write::write_all(&mut std::io::stdout(), &http.decoded_body()?);
    }
    Ok(())
}

fn write_payload(dir: &str, name: &str, bytes: &[u8]) -> Result<(), Error> {
    std::fs::create_dir_all(dir)?;
    let safe = name.replace(['/', ':', '?', '&', '='], "_");
    let path = std::path::Path::new(dir).join(safe);
    std::fs::write(path, bytes)?;
    Ok(())
}

async fn dump_urls(client: &Ccdl, q: &UrlQuery) -> Result<(), Error> {
    for crawl in client.registry().resolve(&q.crawls).await? {
        println!("{}", ccdl::index::compile_cdx_url(&crawl, q, &[]));
    }
    Ok(())
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
