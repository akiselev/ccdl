//! Compile a `UrlQuery` into columnar predicates for the cc-index table.

use crate::model::query::{Filter, MatchType, StatusPred, UrlQuery};
use crate::surt::{surt, surt_prefix_bounds};

/// A set of predicates over the cc-index Parquet columns.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TablePredicates {
    /// Exact `url_host_name` (host/domain queries).
    pub host: Option<String>,
    /// `url_host_tld` equality.
    pub tld: Option<String>,
    /// Half-open `url_surtkey` range `[lo, hi)` for prefix/exact queries.
    pub surt_range: Option<(String, String)>,
    /// `fetch_status` predicate.
    pub status: Option<StatusPred>,
    /// `content_mime_detected` (or declared) equality.
    pub mime: Option<String>,
    /// A residual regex applied per-row (not pushable to statistics).
    pub url_regex: Option<String>,
}

/// Compile the query into columnar predicates.
#[must_use]
pub fn compile_predicates(query: &UrlQuery) -> TablePredicates {
    let mut p = TablePredicates::default();

    match query.match_type {
        MatchType::Host => p.host = Some(host_of(&query.pattern)),
        MatchType::Domain => {
            // Domain covers host + subdomains; use the registrable host as a
            // SURT prefix bound so subdomains sort within range.
            p.surt_range = Some(surt_prefix_bounds(&query.pattern));
            p.tld = tld_of(&query.pattern);
        }
        MatchType::Prefix => {
            p.surt_range = Some(surt_prefix_bounds(&query.pattern));
        }
        MatchType::Exact => {
            let key = surt(&query.pattern);
            let mut hi = key.clone();
            hi.push('\u{0}');
            p.surt_range = Some((key, hi));
        }
    }

    for f in &query.filters {
        match f {
            Filter::Status(s) => p.status = Some(s.clone()),
            Filter::Mime(m) => p.mime = Some(m.clone()),
            Filter::Tld(t) => p.tld = Some(t.clone()),
            Filter::UrlRegex(r) => p.url_regex = Some(r.clone()),
            Filter::Language(_) => {}
        }
    }
    p
}

fn host_of(pattern: &str) -> String {
    let s = pattern.split_once("://").map_or(pattern, |(_, r)| r);
    let s = s.strip_prefix("www.").unwrap_or(s);
    s.split('/').next().unwrap_or(s).to_ascii_lowercase()
}

fn tld_of(pattern: &str) -> Option<String> {
    let host = host_of(pattern);
    host.rsplit('.').next().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_compiles_surt_range() {
        let q = UrlQuery::prefix("example.com/manufacturer/").status(200);
        let p = compile_predicates(&q);
        let (lo, hi) = p.surt_range.clone().unwrap();
        assert!(lo.starts_with("com,example)/manufacturer/"));
        assert!(lo < hi);
        assert_eq!(p.status, Some(StatusPred::Eq(200)));
    }

    #[test]
    fn host_compiles_host_eq() {
        let q = UrlQuery::host("example.com/");
        let p = compile_predicates(&q);
        assert_eq!(p.host.as_deref(), Some("example.com"));
    }

    #[test]
    fn exact_is_tight_range() {
        let q = UrlQuery::exact("example.com/a");
        let p = compile_predicates(&q);
        let (lo, hi) = p.surt_range.unwrap();
        assert_eq!(lo, "com,example)/a");
        assert!(hi > lo);
    }
}
