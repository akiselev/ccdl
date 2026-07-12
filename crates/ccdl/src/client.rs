//! The `Ccdl` facade wiring registry, cache, politeness, transport, and index.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;

use crate::cache::{Cache, FsCache};
use crate::error::{Error, Result};
use crate::http::{HttpTransport, Transport};
use crate::index::{CaptureStream, Estimate, IndexClient};
use crate::model::crawl::CrawlSelector;
use crate::model::query::{MatchType, UrlQuery};
use crate::polite::{Politeness, RetryPolicy};
use crate::registry::{CrawlInfo, Registry};

/// Builder for [`Ccdl`].
pub struct CcdlBuilder {
    cache_dir: Option<PathBuf>,
    max_rps: u32,
    max_concurrency: usize,
    user_agent: Option<String>,
    registry_ttl: Duration,
}

impl Default for CcdlBuilder {
    fn default() -> Self {
        CcdlBuilder {
            cache_dir: None,
            max_rps: 2,
            max_concurrency: 4,
            user_agent: None,
            registry_ttl: Duration::from_secs(24 * 3600),
        }
    }
}

impl CcdlBuilder {
    /// Set the cache directory.
    #[must_use]
    pub fn cache_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cache_dir = Some(dir.into());
        self
    }
    /// Set max requests per second per host bucket.
    #[must_use]
    pub fn max_rps(mut self, rps: u32) -> Self {
        self.max_rps = rps;
        self
    }
    /// Set max concurrent requests.
    #[must_use]
    pub fn max_concurrency(mut self, n: usize) -> Self {
        self.max_concurrency = n;
        self
    }
    /// Set the User-Agent (required; must be descriptive).
    #[must_use]
    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent = Some(ua.into());
        self
    }
    /// Set the registry TTL.
    #[must_use]
    pub fn registry_ttl(mut self, ttl: Duration) -> Self {
        self.registry_ttl = ttl;
        self
    }

    fn default_cache_dir() -> PathBuf {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
            .unwrap_or_else(|| PathBuf::from(".cache"))
            .join("ccdl")
    }

    /// Build the facade.
    pub fn build(self) -> Result<Ccdl> {
        let ua = self
            .user_agent
            .ok_or_else(|| Error::Config("user_agent is required".into()))?;
        let polite = Arc::new(Politeness::new(
            self.max_rps,
            self.max_concurrency,
            RetryPolicy::default(),
            ua,
        )?);
        let cache_dir = self.cache_dir.unwrap_or_else(Self::default_cache_dir);
        let cache: Arc<dyn Cache> = Arc::new(FsCache::new(cache_dir)?);
        let transport: Arc<dyn Transport> =
            Arc::new(HttpTransport::new(polite.clone(), cache.clone())?);
        let registry = Arc::new(Registry::new(transport.clone()));
        let index = IndexClient::new(transport.clone());
        #[cfg(feature = "warc")]
        let warc = crate::warc::WarcClient::new(transport.clone(), cache.clone());
        Ok(Ccdl {
            registry,
            index,
            transport,
            #[cfg(feature = "warc")]
            warc,
        })
    }
}

/// The high-level Common Crawl client.
pub struct Ccdl {
    registry: Arc<Registry>,
    index: IndexClient,
    transport: Arc<dyn Transport>,
    #[cfg(feature = "warc")]
    warc: crate::warc::WarcClient,
}

impl Ccdl {
    /// Start building a client.
    #[must_use]
    pub fn builder() -> CcdlBuilder {
        CcdlBuilder::default()
    }

    /// Access the registry.
    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Access the underlying transport (used by M2 fetch).
    #[must_use]
    pub fn transport(&self) -> &Arc<dyn Transport> {
        &self.transport
    }

    /// List all known crawls.
    pub async fn crawls(&self) -> Result<Vec<CrawlInfo>> {
        self.registry.all().await
    }

    async fn resolve(&self, selector: &CrawlSelector) -> Result<Vec<CrawlInfo>> {
        self.registry.resolve(selector).await
    }

    /// Search the index, returning a lazy capture stream.
    pub async fn search(&self, query: UrlQuery) -> Result<CaptureStream> {
        let crawls = self.resolve(&query.crawls).await?;
        Ok(self.index.search(crawls, query))
    }

    /// Estimate the size of a query.
    pub async fn stats(&self, query: &UrlQuery) -> Result<Estimate> {
        let crawls = self.resolve(&query.crawls).await?;
        self.index.count(&crawls, query).await
    }

    /// Bulk search via the columnar backend (reads cc-index Parquet directly).
    ///
    /// Suited to large prefix/domain queries across many crawls; falls back is
    /// the caller's choice (use [`Ccdl::search`] for CDX).
    #[cfg(feature = "table")]
    pub async fn bulk(&self, query: UrlQuery) -> Result<CaptureStream> {
        let crawls = self.resolve(&query.crawls).await?;
        let table = crate::table::TableClient::http()?;
        table.search(crawls, query).await
    }

    /// Build a manifest for a query, applying a sampling policy per URL.
    ///
    /// This buffers the full result to group and sample by `urlkey`; use
    /// [`Ccdl::search`] for unbounded streaming.
    pub async fn manifest(
        &self,
        query: UrlQuery,
        sampling: crate::model::manifest::Sampling,
    ) -> Result<crate::model::manifest::Manifest> {
        let mut stream = self.search(query.clone()).await?;
        let mut captures = Vec::new();
        while let Some(item) = stream.next().await {
            captures.push(item?);
        }
        let sampled = crate::model::sampling::sample(captures, sampling);
        Ok(crate::model::manifest::Manifest {
            query,
            captures: sampled,
        })
    }

    /// Enumerate a template's literal prefix and bind each matching URL,
    /// returning `(capture, vars)` pairs for URLs the template matches.
    pub async fn enumerate_template(
        &self,
        tmpl: &crate::model::template::UrlTemplate,
    ) -> Result<
        Vec<(
            crate::model::capture::Capture,
            std::collections::BTreeMap<String, String>,
        )>,
    > {
        let query = UrlQuery::prefix(tmpl.prefix());
        let mut stream = self.search(query).await?;
        let mut out = Vec::new();
        while let Some(item) = stream.next().await {
            let cap = item?;
            let stripped = cap
                .url
                .split_once("://")
                .map_or(cap.url.as_str(), |(_, rest)| rest);
            let stripped = stripped.strip_prefix("www.").unwrap_or(stripped);
            if let Some(vars) = tmpl.bind(stripped) {
                out.push((cap, vars));
            }
        }
        Ok(out)
    }

    /// Fetch and parse a capture's WARC record (payload cached by digest).
    #[cfg(feature = "warc")]
    pub async fn fetch(
        &self,
        capture: &crate::model::capture::Capture,
    ) -> Result<crate::warc::WarcRecord> {
        self.warc.fetch(capture).await
    }

    /// Fetch a capture and parse the enclosed HTTP response.
    #[cfg(feature = "warc")]
    pub async fn fetch_http(
        &self,
        capture: &crate::model::capture::Capture,
    ) -> Result<crate::warc::HttpCapture> {
        self.warc.fetch_http(capture).await
    }

    /// Resolve the newest 200 capture for a URL and return its decoded text.
    #[cfg(feature = "warc")]
    pub async fn text(&self, url: &str) -> Result<String> {
        let query = UrlQuery::exact(url).status(200).crawls(CrawlSelector::All);
        let mut stream = self.search(query).await?;
        let mut newest: Option<crate::model::capture::Capture> = None;
        while let Some(item) = stream.next().await {
            let cap = item?;
            if newest.as_ref().is_none_or(|n| cap.timestamp > n.timestamp) {
                newest = Some(cap);
            }
        }
        let capture = newest.ok_or(Error::NotFound)?;
        self.fetch_http(&capture).await?.text()
    }

    /// Fetch a URL from a specific crawl selector, returning the HTTP capture.
    #[cfg(feature = "warc")]
    pub async fn fetch_url(
        &self,
        url: &str,
        crawls: CrawlSelector,
    ) -> Result<crate::warc::HttpCapture> {
        let query = UrlQuery::exact(url).status(200).crawls(crawls);
        let mut stream = self.search(query).await?;
        let cap = match stream.next().await {
            Some(item) => item?,
            None => return Err(Error::NotFound),
        };
        self.fetch_http(&cap).await
    }

    /// Begin an enumeration for a pattern.
    #[must_use]
    pub fn enumerate(
        &self,
        pattern: impl Into<String>,
        match_type: MatchType,
    ) -> EnumerateBuilder<'_> {
        EnumerateBuilder {
            client: self,
            query: UrlQuery {
                pattern: pattern.into(),
                match_type,
                crawls: CrawlSelector::default(),
                filters: Vec::new(),
                collapse: None,
                fields: None,
                time_range: None,
                limit: None,
            },
            distinct: false,
        }
    }
}

/// Fluent enumeration builder.
pub struct EnumerateBuilder<'a> {
    client: &'a Ccdl,
    query: UrlQuery,
    distinct: bool,
}

impl EnumerateBuilder<'_> {
    /// Deduplicate by `urlkey` across the whole stream (bounded by a hash set;
    /// the memory ceiling is the number of distinct urlkeys — revisit with the
    /// columnar backend in M3).
    #[must_use]
    pub fn distinct_urls(mut self) -> Self {
        self.distinct = true;
        self
    }

    /// Set the crawl selector.
    #[must_use]
    pub fn crawls(mut self, sel: CrawlSelector) -> Self {
        self.query.crawls = sel;
        self
    }

    /// Require an exact status.
    #[must_use]
    pub fn status(mut self, status: u16) -> Self {
        self.query = self.query.status(status);
        self
    }

    /// The compiled query (for `--dry-run`).
    #[must_use]
    pub fn query(&self) -> &UrlQuery {
        &self.query
    }

    /// Execute, returning a capture stream (deduped if requested).
    pub async fn run(self) -> Result<CaptureStream> {
        let stream = self.client.search(self.query).await?;
        if !self.distinct {
            return Ok(stream);
        }
        let deduped = async_stream::try_stream! {
            let mut seen: HashSet<String> = HashSet::new();
            futures::pin_mut!(stream);
            while let Some(item) = stream.next().await {
                let cap = item?;
                if seen.insert(cap.urlkey.clone()) {
                    yield cap;
                }
            }
        };
        Ok(Box::pin(deduped))
    }
}
