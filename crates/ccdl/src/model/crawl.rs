//! Crawl identifiers and selectors.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::Error;

/// A monthly crawl, e.g. `CC-MAIN-2026-30`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CrawlId(pub String);

impl fmt::Display for CrawlId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for CrawlId {
    fn from(s: &str) -> Self {
        CrawlId(s.to_owned())
    }
}

/// How to select which crawls a query spans.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CrawlSelector {
    /// Every known crawl.
    All,
    /// The single most recent crawl.
    #[default]
    Latest,
    /// The `n` most recent crawls.
    LatestN(usize),
    /// An explicit list of crawl ids.
    Ids(Vec<CrawlId>),
    /// Crawls whose end date is on or after the given date.
    Since(chrono::NaiveDate),
    /// Crawls whose year falls within `[from, to]` inclusive.
    YearRange {
        /// First year (inclusive).
        from: u16,
        /// Last year (inclusive).
        to: u16,
    },
}

impl fmt::Display for CrawlSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CrawlSelector::All => f.write_str("all"),
            CrawlSelector::Latest => f.write_str("latest"),
            CrawlSelector::LatestN(n) => write!(f, "latest-{n}"),
            CrawlSelector::Ids(ids) => {
                let joined: Vec<_> = ids.iter().map(|i| i.0.as_str()).collect();
                f.write_str(&joined.join(","))
            }
            CrawlSelector::Since(d) => write!(f, "since:{d}"),
            CrawlSelector::YearRange { from, to } => write!(f, "{from}..{to}"),
        }
    }
}

impl FromStr for CrawlSelector {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("all") {
            return Ok(CrawlSelector::All);
        }
        if s.eq_ignore_ascii_case("latest") {
            return Ok(CrawlSelector::Latest);
        }
        if let Some(rest) = s.strip_prefix("latest-") {
            let n = rest
                .parse::<usize>()
                .map_err(|_| Error::Config(format!("invalid latest-N selector: {s}")))?;
            return Ok(CrawlSelector::LatestN(n));
        }
        if let Some(rest) = s.strip_prefix("since:") {
            let d = rest
                .parse::<chrono::NaiveDate>()
                .map_err(|_| Error::Config(format!("invalid since date: {rest}")))?;
            return Ok(CrawlSelector::Since(d));
        }
        if let Some((a, b)) = s.split_once("..") {
            let from = a
                .parse::<u16>()
                .map_err(|_| Error::Config(format!("invalid year range: {s}")))?;
            let to = b
                .parse::<u16>()
                .map_err(|_| Error::Config(format!("invalid year range: {s}")))?;
            return Ok(CrawlSelector::YearRange { from, to });
        }
        if s.contains("CC-MAIN") || s.contains(',') {
            let ids = s.split(',').map(|p| CrawlId(p.trim().to_owned())).collect();
            return Ok(CrawlSelector::Ids(ids));
        }
        Err(Error::Config(format!("unrecognized crawl selector: {s}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_roundtrips() {
        for s in ["all", "latest", "latest-6", "2020..2024"] {
            let sel: CrawlSelector = s.parse().unwrap();
            assert_eq!(sel.to_string(), s);
        }
    }

    #[test]
    fn parse_ids() {
        let sel: CrawlSelector = "CC-MAIN-2026-30".parse().unwrap();
        assert_eq!(
            sel,
            CrawlSelector::Ids(vec![CrawlId("CC-MAIN-2026-30".into())])
        );
    }

    #[test]
    fn parse_since() {
        let sel: CrawlSelector = "since:2021-01-01".parse().unwrap();
        assert!(matches!(sel, CrawlSelector::Since(_)));
    }
}
