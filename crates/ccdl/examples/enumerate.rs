//! Enumerate captures for a URL prefix from Common Crawl and print them.
//!
//! Run against live Common Crawl:
//!
//! ```console
//! $ cargo run --example enumerate -- 'en.wikipedia.org/wiki/'
//! ```
//!
//! Pass a URL prefix as the first argument (defaults to a sample prefix).
//! This example makes real, rate-limited network requests, so it is compiled by
//! CI but not executed there.

use ccdl::model::crawl::CrawlSelector;
use ccdl::prelude::*;
use futures::StreamExt;

#[tokio::main]
async fn main() -> ccdl::Result<()> {
    let pattern = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "en.wikipedia.org/wiki/".to_owned());

    let ccdl = Ccdl::builder()
        .user_agent("ccdl-example/1.0 (+https://github.com/akiselev/ccdl)")
        .max_rps(2)
        .build()?;

    let query = UrlQuery::prefix(pattern)
        .status(200)
        .crawls(CrawlSelector::LatestN(1))
        .limit(10);

    let mut stream = ccdl.search(query).await?;
    let mut count = 0usize;
    while let Some(capture) = stream.next().await {
        let c = capture?;
        println!(
            "{}  {}  {}",
            c.timestamp.format("%Y-%m-%d"),
            c.status,
            c.url
        );
        count += 1;
    }
    eprintln!("{count} captures");
    Ok(())
}
