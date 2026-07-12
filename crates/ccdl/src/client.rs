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
        Ok(Ccdl {
            registry,
            index,
            transport,
        })
    }
}

/// The high-level Common Crawl client.
pub struct Ccdl {
    registry: Arc<Registry>,
    index: IndexClient,
    transport: Arc<dyn Transport>,
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
