//! The crawl registry: loads and caches `collinfo.json`.

use std::sync::Arc;

use serde::Deserialize;
use tokio::sync::RwLock;

use crate::error::{Error, Result};
use crate::http::Transport;
use crate::model::crawl::{CrawlId, CrawlSelector};

/// The public listing of all Common Crawl indexes.
pub const COLLINFO_URL: &str = "https://index.commoncrawl.org/collinfo.json";

/// Metadata for one crawl.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrawlInfo {
    /// Crawl id, e.g. `CC-MAIN-2026-30`.
    pub id: CrawlId,
    /// Human-readable name.
    pub name: String,
    /// The CDX API endpoint for this crawl.
    pub cdx_api: String,
    /// Coverage start (raw string from `collinfo`).
    pub from: Option<String>,
    /// Coverage end (raw string from `collinfo`).
    pub to: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawInfo {
    id: String,
    name: String,
    #[serde(rename = "cdx-api")]
    cdx_api: String,
    from: Option<String>,
    to: Option<String>,
}

impl CrawlInfo {
    fn year(&self) -> Option<u16> {
        // ids look like CC-MAIN-2026-30; grab the 4-digit year.
        self.id.0.split('-').find_map(|p| {
            if p.len() == 4 {
                p.parse::<u16>().ok()
            } else {
                None
            }
        })
    }
}

/// Loads, caches, and resolves crawl selectors.
pub struct Registry {
    transport: Arc<dyn Transport>,
    crawls: RwLock<Option<Vec<CrawlInfo>>>,
}

impl Registry {
    /// Create a registry over a transport.
    #[must_use]
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Registry {
            transport,
            crawls: RwLock::new(None),
        }
    }

    /// Return all crawls, loading (and caching in memory) on first use.
    /// Crawls are returned newest-first (as `collinfo.json` provides them).
    pub async fn all(&self) -> Result<Vec<CrawlInfo>> {
        if let Some(c) = self.crawls.read().await.as_ref() {
            return Ok(c.clone());
        }
        let loaded = self.load().await?;
        *self.crawls.write().await = Some(loaded.clone());
        Ok(loaded)
    }

    /// Force a reload of the crawl list.
    pub async fn refresh(&self) -> Result<()> {
        let loaded = self.load().await?;
        *self.crawls.write().await = Some(loaded);
        Ok(())
    }

    async fn load(&self) -> Result<Vec<CrawlInfo>> {
        let json = self.transport.get_json(COLLINFO_URL).await?;
        Self::parse(&json)
    }

    fn parse(json: &serde_json::Value) -> Result<Vec<CrawlInfo>> {
        let raw: Vec<RawInfo> =
            serde_json::from_value(json.clone()).map_err(|e| Error::Parse(e.to_string()))?;
        Ok(raw
            .into_iter()
            .map(|r| CrawlInfo {
                id: CrawlId(r.id),
                name: r.name,
                cdx_api: r.cdx_api,
                from: r.from,
                to: r.to,
            })
            .collect())
    }

    /// Resolve a selector against the crawl list.
    pub async fn resolve(&self, selector: &CrawlSelector) -> Result<Vec<CrawlInfo>> {
        let all = self.all().await?;
        Ok(Self::resolve_in(&all, selector))
    }

    fn resolve_in(all: &[CrawlInfo], selector: &CrawlSelector) -> Vec<CrawlInfo> {
        match selector {
            CrawlSelector::All => all.to_vec(),
            CrawlSelector::Latest => all.iter().take(1).cloned().collect(),
            CrawlSelector::LatestN(n) => all.iter().take(*n).cloned().collect(),
            CrawlSelector::Ids(ids) => all
                .iter()
                .filter(|c| ids.contains(&c.id))
                .cloned()
                .collect(),
            CrawlSelector::YearRange { from, to } => all
                .iter()
                .filter(|c| c.year().is_some_and(|y| y >= *from && y <= *to))
                .cloned()
                .collect(),
            CrawlSelector::Since(date) => {
                let from_year =
                    u16::try_from(date.format("%Y").to_string().parse::<i32>().unwrap_or(0))
                        .unwrap_or(0);
                all.iter()
                    .filter(|c| c.year().is_some_and(|y| y >= from_year))
                    .cloned()
                    .collect()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<CrawlInfo> {
        let json = serde_json::json!([
            {"id":"CC-MAIN-2026-30","name":"July 2026","cdx-api":"https://index.commoncrawl.org/CC-MAIN-2026-30-index"},
            {"id":"CC-MAIN-2023-10","name":"March 2023","cdx-api":"https://index.commoncrawl.org/CC-MAIN-2023-10-index"},
            {"id":"CC-MAIN-2020-05","name":"Jan 2020","cdx-api":"https://index.commoncrawl.org/CC-MAIN-2020-05-index"}
        ]);
        Registry::parse(&json).unwrap()
    }

    #[test]
    fn resolve_latest_n() {
        let all = fixture();
        let r = Registry::resolve_in(&all, &CrawlSelector::LatestN(2));
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].id.0, "CC-MAIN-2026-30");
    }

    #[test]
    fn resolve_year_range() {
        let all = fixture();
        let r = Registry::resolve_in(
            &all,
            &CrawlSelector::YearRange {
                from: 2020,
                to: 2023,
            },
        );
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn resolve_ids() {
        let all = fixture();
        let r = Registry::resolve_in(
            &all,
            &CrawlSelector::Ids(vec![CrawlId("CC-MAIN-2020-05".into())]),
        );
        assert_eq!(r.len(), 1);
    }
}
