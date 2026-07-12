//! Parse CDXJ (JSON-lines) rows into `Capture`.

use chrono::{TimeZone, Utc};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::model::capture::Capture;
use crate::model::crawl::CrawlId;

fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn parse_ts(raw: &str) -> Result<chrono::DateTime<Utc>> {
    // CDX timestamps are YYYYMMDDHHMMSS (UTC).
    let dt = chrono::NaiveDateTime::parse_from_str(raw, "%Y%m%d%H%M%S")
        .map_err(|_| Error::Parse(format!("bad timestamp: {raw}")))?;
    Ok(Utc.from_utc_datetime(&dt))
}

/// Parse a single CDXJ line into a `Capture`, tagging it with `crawl`.
pub fn parse_cdxj_line(crawl: &CrawlId, line: &str) -> Result<Capture> {
    let v: Value = serde_json::from_str(line).map_err(|e| Error::Parse(e.to_string()))?;
    let timestamp =
        parse_ts(&s(&v, "timestamp").ok_or_else(|| Error::Parse("no timestamp".into()))?)?;
    let status = s(&v, "status")
        .and_then(|x| x.parse::<u16>().ok())
        .unwrap_or(0);
    let length = s(&v, "length").and_then(|x| x.parse().ok()).unwrap_or(0);
    let offset = s(&v, "offset").and_then(|x| x.parse().ok()).unwrap_or(0);
    let languages = s(&v, "languages")
        .map(|l| {
            l.split(',')
                .map(str::to_owned)
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default();

    Ok(Capture {
        crawl: crawl.clone(),
        urlkey: s(&v, "urlkey").unwrap_or_default(),
        url: s(&v, "url").unwrap_or_default(),
        timestamp,
        status,
        mime: s(&v, "mime"),
        mime_detected: s(&v, "mime-detected"),
        languages,
        digest: s(&v, "digest").unwrap_or_default(),
        length,
        offset,
        filename: s(&v, "filename").unwrap_or_default(),
        redirect: s(&v, "redirect"),
        truncated: s(&v, "truncated"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_row() {
        let line = r#"{"urlkey":"com,example)/x","timestamp":"20260701120000","url":"https://example.com/x","mime":"text/html","mime-detected":"text/html","status":"200","digest":"sha1:ABC","length":"1234","offset":"5678","filename":"crawl-data/foo.warc.gz","languages":"eng"}"#;
        let c = parse_cdxj_line(&CrawlId("CC-MAIN-2026-30".into()), line).unwrap();
        assert_eq!(c.status, 200);
        assert_eq!(c.length, 1234);
        assert_eq!(c.offset, 5678);
        assert_eq!(c.languages, vec!["eng".to_string()]);
        assert_eq!(c.range_header(), "bytes=5678-6911");
    }
}
