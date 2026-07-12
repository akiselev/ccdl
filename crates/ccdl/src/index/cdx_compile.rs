//! Compile a `UrlQuery` + crawl endpoint into a CDX request URL.

use crate::model::query::{Filter, StatusPred, UrlQuery};
use crate::registry::CrawlInfo;

/// Percent-encode a query-string value (conservative allow-list).
fn enc(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

fn filter_params(query: &UrlQuery, params: &mut Vec<(String, String)>) {
    for f in &query.filters {
        match f {
            Filter::Status(StatusPred::Eq(s)) => {
                params.push(("filter".into(), format!("status:{s}")));
            }
            Filter::Status(StatusPred::In(codes)) => {
                let alt = codes
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("|");
                params.push(("filter".into(), format!("~status:({alt})")));
            }
            Filter::Status(StatusPred::Class(c)) => {
                params.push(("filter".into(), format!("~status:{c}..")));
            }
            Filter::Mime(m) => params.push(("filter".into(), format!("mime:{m}"))),
            Filter::Language(l) => params.push(("filter".into(), format!("languages:{l}"))),
            Filter::Tld(t) => params.push(("filter".into(), format!("urlkey:{t},.*"))),
            Filter::UrlRegex(r) => params.push(("filter".into(), format!("~url:{r}"))),
        }
    }
}

/// Build the CDX API URL for a query against one crawl. Extra params (e.g.
/// `page`, `showNumPages`) are appended verbatim.
#[must_use]
pub fn compile_cdx_url(crawl: &CrawlInfo, query: &UrlQuery, extra: &[(&str, String)]) -> String {
    let mut params: Vec<(String, String)> = Vec::new();
    params.push(("url".into(), query.pattern.clone()));
    params.push(("matchType".into(), query.match_type.cdx_token().into()));
    params.push(("output".into(), "json".into()));
    filter_params(query, &mut params);
    if let Some(c) = &query.collapse {
        params.push(("collapse".into(), c.cdx_token()));
    }
    if let Some(fields) = &query.fields {
        let fl = fields
            .iter()
            .map(|f| f.cdx_token())
            .collect::<Vec<_>>()
            .join(",");
        params.push(("fl".into(), fl));
    }
    if let Some((from, to)) = &query.time_range {
        params.push(("from".into(), from.format("%Y%m%d%H%M%S").to_string()));
        params.push(("to".into(), to.format("%Y%m%d%H%M%S").to_string()));
    }
    if let Some(limit) = query.limit {
        params.push(("limit".into(), limit.to_string()));
    }
    for (k, v) in extra {
        params.push(((*k).to_owned(), v.clone()));
    }

    let qs = params
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{}?{qs}", crawl.cdx_api)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::crawl::CrawlId;
    use crate::model::query::UrlQuery;

    fn crawl() -> CrawlInfo {
        CrawlInfo {
            id: CrawlId("CC-MAIN-2026-30".into()),
            name: "x".into(),
            cdx_api: "https://index.commoncrawl.org/CC-MAIN-2026-30-index".into(),
            from: None,
            to: None,
        }
    }

    #[test]
    fn golden_url() {
        let q = UrlQuery::prefix("example.com/manufacturer/").status(200);
        let url = compile_cdx_url(&crawl(), &q, &[("showNumPages", "true".into())]);
        assert!(url.starts_with("https://index.commoncrawl.org/CC-MAIN-2026-30-index?"));
        assert!(url.contains("matchType=prefix"));
        assert!(url.contains("filter=status%3A200"));
        assert!(url.contains("output=json"));
        assert!(url.contains("showNumPages=true"));
    }
}
