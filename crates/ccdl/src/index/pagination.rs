//! Streaming, lazy pagination over the CDX API.

use std::pin::Pin;
use std::sync::Arc;

use futures::Stream;

use super::cdx_compile::compile_cdx_url;
use super::cdx_parse::parse_cdxj_line;
use crate::error::{Error, Result};
use crate::http::Transport;
use crate::model::capture::Capture;
use crate::model::query::UrlQuery;
use crate::registry::CrawlInfo;

/// A lazy stream of captures.
pub type CaptureStream = Pin<Box<dyn Stream<Item = Result<Capture>> + Send>>;

/// A rough size estimate for a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Estimate {
    /// Number of index pages the query spans (summed across crawls).
    pub pages: u64,
}

/// The CDX index client.
pub struct IndexClient {
    transport: Arc<dyn Transport>,
    /// Guard: refuse queries wider than this many pages (per crawl).
    pub max_pages: u64,
}

impl IndexClient {
    /// Create a client over a transport.
    #[must_use]
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        IndexClient {
            transport,
            max_pages: 1000,
        }
    }

    /// Probe `showNumPages` for one crawl.
    async fn num_pages(
        transport: &Arc<dyn Transport>,
        crawl: &CrawlInfo,
        query: &UrlQuery,
    ) -> Result<u64> {
        let url = compile_cdx_url(crawl, query, &[("showNumPages", "true".into())]);
        let text = match transport.get_text(&url).await {
            Ok(t) => t,
            Err(Error::NotFound) => return Ok(0),
            Err(e) => return Err(e),
        };
        let trimmed = text.trim();
        // Response may be a bare number or JSON like {"pages":N,...}.
        if let Ok(n) = trimmed.parse::<u64>() {
            return Ok(n);
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(n) = v.get("pages").and_then(serde_json::Value::as_u64) {
                return Ok(n);
            }
        }
        Ok(0)
    }

    /// Estimate query size across the given crawls.
    pub async fn count(&self, crawls: &[CrawlInfo], query: &UrlQuery) -> Result<Estimate> {
        let mut pages = 0;
        for crawl in crawls {
            pages += Self::num_pages(&self.transport, crawl, query).await?;
        }
        Ok(Estimate { pages })
    }

    /// Stream captures across crawls, paging lazily, ordered by crawl.
    #[must_use]
    pub fn search(&self, crawls: Vec<CrawlInfo>, query: UrlQuery) -> CaptureStream {
        let transport = self.transport.clone();
        let max_pages = self.max_pages;
        Box::pin(async_stream::try_stream! {
            for crawl in crawls {
                let pages = IndexClient::num_pages(&transport, &crawl, &query).await?;
                if pages > max_pages {
                    Err(Error::IndexTooLarge { pages })?;
                }
                let total = if pages == 0 { 1 } else { pages };
                for page in 0..total {
                    let url = compile_cdx_url(&crawl, &query, &[("page", page.to_string())]);
                    let text = match transport.get_text(&url).await {
                        Ok(t) => t,
                        Err(Error::NotFound) => continue,
                        Err(e) => Err(e)?,
                    };
                    for line in text.lines() {
                        let line = line.trim();
                        if line.is_empty() { continue; }
                        let cap = parse_cdxj_line(&crawl.id, line)?;
                        yield cap;
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::MockTransport;
    use crate::model::crawl::CrawlId;
    use futures::StreamExt;

    fn crawl() -> CrawlInfo {
        CrawlInfo {
            id: CrawlId("CC-MAIN-2026-30".into()),
            name: "x".into(),
            cdx_api: "https://index.commoncrawl.org/CC-MAIN-2026-30-index".into(),
            from: None,
            to: None,
        }
    }

    fn row(url: &str) -> String {
        format!(
            r#"{{"urlkey":"k","timestamp":"20260701120000","url":"{url}","status":"200","digest":"sha1:A","length":"10","offset":"0","filename":"f.warc.gz"}}"#
        )
    }

    #[tokio::test]
    async fn paginates_two_pages() {
        let q = UrlQuery::prefix("example.com/");
        let probe = compile_cdx_url(&crawl(), &q, &[("showNumPages", "true".into())]);
        let p0 = compile_cdx_url(&crawl(), &q, &[("page", "0".into())]);
        let p1 = compile_cdx_url(&crawl(), &q, &[("page", "1".into())]);
        let transport = Arc::new(
            MockTransport::new()
                .with(probe, "2")
                .with(p0, row("https://example.com/a"))
                .with(p1, row("https://example.com/b")),
        );
        let client = IndexClient::new(transport);
        let out: Vec<_> = client.search(vec![crawl()], q).collect().await;
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(std::result::Result::is_ok));
    }

    #[tokio::test]
    async fn count_reads_num_pages() {
        let q = UrlQuery::prefix("example.com/");
        let probe = compile_cdx_url(&crawl(), &q, &[("showNumPages", "true".into())]);
        let transport = Arc::new(MockTransport::new().with(probe, "7"));
        let client = IndexClient::new(transport);
        let est = client.count(&[crawl()], &q).await.unwrap();
        assert_eq!(est.pages, 7);
    }
}
