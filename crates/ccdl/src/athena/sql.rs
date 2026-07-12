//! Compile a `UrlQuery` into Athena SQL over the `ccindex` table.

use super::AthenaConfig;
use crate::model::crawl::CrawlId;
use crate::model::query::{Filter, MatchType, StatusPred, UrlQuery};
use crate::surt::surt_prefix_bounds;

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn host_of(pattern: &str) -> String {
    let s = pattern.split_once("://").map_or(pattern, |(_, r)| r);
    let s = s.strip_prefix("www.").unwrap_or(s);
    s.split('/').next().unwrap_or(s).to_ascii_lowercase()
}

/// Compile a query (restricted to explicit crawl ids) into an Athena SQL string.
#[must_use]
pub fn compile_sql(cfg: &AthenaConfig, query: &UrlQuery, crawls: &[CrawlId]) -> String {
    let mut wheres: Vec<String> = Vec::new();
    wheres.push("subset = 'warc'".to_owned());

    if !crawls.is_empty() {
        let list = crawls
            .iter()
            .map(|c| quote(&c.0))
            .collect::<Vec<_>>()
            .join(", ");
        wheres.push(format!("crawl IN ({list})"));
    }

    match query.match_type {
        MatchType::Host => {
            wheres.push(format!(
                "url_host_name = {}",
                quote(&host_of(&query.pattern))
            ));
        }
        MatchType::Domain => {
            let host = host_of(&query.pattern);
            wheres.push(format!(
                "(url_host_name = {h} OR url_host_name LIKE {sub})",
                h = quote(&host),
                sub = quote(&format!("%.{host}"))
            ));
        }
        MatchType::Prefix => {
            let (lo, hi) = surt_prefix_bounds(&query.pattern);
            wheres.push(format!(
                "url_surtkey >= {} AND url_surtkey < {}",
                quote(&lo),
                quote(&hi)
            ));
        }
        MatchType::Exact => {
            wheres.push(format!("url = {}", quote(&query.pattern)));
        }
    }

    for f in &query.filters {
        match f {
            Filter::Status(StatusPred::Eq(s)) => wheres.push(format!("fetch_status = {s}")),
            Filter::Status(StatusPred::In(codes)) => {
                let list = codes
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                wheres.push(format!("fetch_status IN ({list})"));
            }
            Filter::Status(StatusPred::Class(c)) => {
                let lo = u16::from(*c) * 100;
                wheres.push(format!(
                    "fetch_status >= {lo} AND fetch_status < {}",
                    lo + 100
                ));
            }
            Filter::Mime(m) => wheres.push(format!("content_mime_detected = {}", quote(m))),
            Filter::Language(l) => wheres.push(format!(
                "content_languages LIKE {}",
                quote(&format!("%{l}%"))
            )),
            Filter::Tld(t) => wheres.push(format!("url_host_tld = {}", quote(t))),
            Filter::UrlRegex(r) => wheres.push(format!("REGEXP_LIKE(url, {})", quote(r))),
        }
    }

    let cols = "url_surtkey, url, fetch_time, fetch_status, content_mime_type, \
                content_mime_detected, content_languages, content_digest, \
                warc_filename, warc_record_offset, warc_record_length, crawl";
    let mut sql = format!(
        "SELECT {cols} FROM {tbl} WHERE {w}",
        tbl = cfg.table,
        w = wheres.join(" AND ")
    );
    if let Some(limit) = query.limit {
        use std::fmt::Write as _;
        let _ = write!(sql, " LIMIT {limit}");
    }
    sql
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_sql_has_surt_range() {
        let cfg = AthenaConfig::default();
        let q = UrlQuery::prefix("example.com/manufacturer/").status(200);
        let sql = compile_sql(&cfg, &q, &[CrawlId("CC-MAIN-2026-30".into())]);
        assert!(sql.contains("subset = 'warc'"));
        assert!(sql.contains("crawl IN ('CC-MAIN-2026-30')"));
        assert!(sql.contains("url_surtkey >= 'com,example)/manufacturer/'"));
        assert!(sql.contains("fetch_status = 200"));
    }

    #[test]
    fn host_sql() {
        let cfg = AthenaConfig::default();
        let q = UrlQuery::host("example.com/");
        let sql = compile_sql(&cfg, &q, &[]);
        assert!(sql.contains("url_host_name = 'example.com'"));
    }

    #[test]
    fn escapes_quotes() {
        let cfg = AthenaConfig::default();
        let q = UrlQuery::exact("a'b");
        let sql = compile_sql(&cfg, &q, &[]);
        assert!(sql.contains("url = 'a''b'"));
    }
}
