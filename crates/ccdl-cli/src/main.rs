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
    /// Discover sitemap seed URLs for a host from Common Crawl captures.
    Sitemap {
        /// The host, e.g. `www.digikey.com`.
        host: String,
    },
    /// Build a sampled manifest for a URL pattern.
    Manifest {
        /// URL pattern.
        pattern: String,
        /// Match type: exact|prefix|host|domain.
        #[arg(long, default_value = "prefix")]
        r#match: String,
        /// Sampling policy: all|unique-digest|first-last|first-last-digest|adaptive.
        #[arg(long, default_value = "first-last-digest")]
        sample: String,
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
        /// Stop after this many records; emits a resume token on exhaustion.
        #[arg(long)]
        budget_records: Option<u64>,
        /// Bind a URL template, e.g. `host/manufacturer/{name}/{part}`.
        #[arg(long)]
        template: Option<String>,
        /// Use the columnar (Parquet) backend instead of CDX.
        #[cfg(feature = "table")]
        #[arg(long)]
        bulk: bool,
        /// Print the compiled query instead of running.
        #[arg(long)]
        dry_run: bool,
    },
}

fn parse_sampling(s: &str) -> ccdl::model::manifest::Sampling {
    use ccdl::model::manifest::Sampling;
    match s {
        "all" => Sampling::All,
        "unique-digest" => Sampling::UniqueDigest,
        "first-last" => Sampling::FirstLast,
        "adaptive" => Sampling::AdaptiveChangePoint { max_per_url: 8 },
        _ => Sampling::FirstLastAndDigestChanges,
    }
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
        Error::RateLimited { .. } | Error::BudgetExhausted { .. } => ExitCode::from(3),
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

#[allow(clippy::too_many_lines)]
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
            cmd_search(&client, q, dry_run, format).await
        }
        Command::Text { url } => {
            let text = client.text(&url).await?;
            println!("{text}");
            Ok(())
        }
        Command::Fetch { target, out, text } => {
            cmd_fetch(&client, &target, out.as_deref(), text).await
        }
        Command::Manifest {
            pattern,
            r#match,
            sample,
        } => {
            let q = base_query(pattern, &r#match, selector);
            let manifest = client.manifest(q, parse_sampling(&sample)).await?;
            let stdout = std::io::stdout();
            let mut out = stdout.lock();
            let mut writer = Writer::new(format);
            for c in &manifest.captures {
                let _ = writer.write(&mut out, c);
            }
            Ok(())
        }
        Command::Sitemap { host } => {
            let seeds = client.sitemap_seeds(&host, selector).await?;
            for url in seeds {
                println!("{url}");
            }
            Ok(())
        }
        Command::Enumerate {
            pattern,
            r#match,
            status,
            distinct,
            budget_records,
            template,
            #[cfg(feature = "table")]
            bulk,
            dry_run,
        } => {
            let opts = EnumOpts {
                pattern,
                match_str: r#match,
                status,
                distinct,
                budget_records,
                template,
                #[cfg(feature = "table")]
                bulk,
                dry_run,
                selector,
                format,
            };
            cmd_enumerate(&client, opts).await
        }
    }
}

struct EnumOpts {
    pattern: String,
    match_str: String,
    status: Option<u16>,
    distinct: bool,
    budget_records: Option<u64>,
    template: Option<String>,
    #[cfg(feature = "table")]
    bulk: bool,
    dry_run: bool,
    selector: CrawlSelector,
    format: Format,
}

async fn cmd_enumerate(client: &Ccdl, o: EnumOpts) -> Result<(), Error> {
    if let Some(tmpl) = o.template {
        let t = ccdl::model::template::UrlTemplate::parse(&tmpl)?;
        for (cap, vars) in client.enumerate_template(&t).await? {
            let obj = serde_json::json!({ "url": cap.url, "vars": vars });
            println!("{obj}");
        }
        return Ok(());
    }
    #[cfg(feature = "table")]
    if o.bulk {
        let q = base_query(o.pattern, &o.match_str, o.selector);
        if o.dry_run {
            return dump_urls(client, &q).await;
        }
        return write_stream(client.bulk(q).await?, o.format).await;
    }
    let mut b = client
        .enumerate(o.pattern, parse_match(&o.match_str))
        .crawls(o.selector);
    if let Some(s) = o.status {
        b = b.status(s);
    }
    if o.distinct {
        b = b.distinct_urls();
    }
    if o.dry_run {
        return dump_urls(client, b.query()).await;
    }
    let stream = b.run().await?;
    let budget = ccdl::budget::Budget {
        max_records: o.budget_records,
        ..Default::default()
    };
    write_stream(budget.apply(stream), o.format).await
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

async fn cmd_search(
    client: &Ccdl,
    q: UrlQuery,
    dry_run: bool,
    format: Format,
) -> Result<(), Error> {
    if dry_run {
        return dump_urls(client, &q).await;
    }
    write_stream(client.search(q).await?, format).await
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
