//! The backend-agnostic capture row.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::crawl::CrawlId;

/// The base URL for Common Crawl WARC payloads (free HTTPS via `CloudFront`).
pub const DATA_HOST: &str = "https://data.commoncrawl.org";

/// One capture row from an index (CDX row or columnar row), backend-agnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capture {
    /// The crawl this capture belongs to.
    pub crawl: CrawlId,
    /// SURT key, e.g. `com,example)/manufacturer/acme/x1`.
    pub urlkey: String,
    /// The original URL.
    pub url: String,
    /// Capture timestamp.
    pub timestamp: DateTime<Utc>,
    /// `fetch_status`.
    pub status: u16,
    /// Declared MIME.
    pub mime: Option<String>,
    /// Sniffed MIME.
    pub mime_detected: Option<String>,
    /// Detected content languages.
    #[serde(default)]
    pub languages: Vec<String>,
    /// Content digest, e.g. `sha1:…`.
    pub digest: String,
    /// `warc_record_length`.
    pub length: u64,
    /// `warc_record_offset`.
    pub offset: u64,
    /// `warc_filename` (relative to the data host).
    pub filename: String,
    /// Redirect target, if any.
    pub redirect: Option<String>,
    /// Truncation reason, if any.
    pub truncated: Option<String>,
}

impl Capture {
    /// The full WARC payload URL.
    #[must_use]
    pub fn warc_url(&self) -> String {
        format!("{DATA_HOST}/{}", self.filename)
    }

    /// The HTTP `Range` header value covering exactly this record.
    #[must_use]
    pub fn range_header(&self) -> String {
        let end = self.offset + self.length - 1;
        format!("bytes={}-{end}", self.offset)
    }

    /// A stable identifier for dedup: crawl + offset + filename are unique.
    #[must_use]
    pub fn capture_key(&self) -> String {
        format!("{}:{}:{}", self.crawl.0, self.filename, self.offset)
    }

    /// Heuristic: does this capture look like a bot-challenge / interstitial?
    #[must_use]
    pub fn looks_like_challenge(&self) -> bool {
        if matches!(self.status, 403 | 429 | 503) {
            return true;
        }
        let u = self.url.to_ascii_lowercase();
        u.contains("/cdn-cgi/") || u.contains("__cf_chl") || u.contains("challenge")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Capture {
        Capture {
            crawl: CrawlId("CC-MAIN-2026-30".into()),
            urlkey: "com,example)/x".into(),
            url: "https://example.com/x".into(),
            timestamp: Utc::now(),
            status: 200,
            mime: Some("text/html".into()),
            mime_detected: Some("text/html".into()),
            languages: vec![],
            digest: "sha1:ABC".into(),
            length: 100,
            offset: 500,
            filename: "crawl-data/foo.warc.gz".into(),
            redirect: None,
            truncated: None,
        }
    }

    #[test]
    fn range_header_is_inclusive() {
        let c = sample();
        assert_eq!(c.range_header(), "bytes=500-599");
    }

    #[test]
    fn warc_url_joins_host() {
        assert_eq!(
            sample().warc_url(),
            "https://data.commoncrawl.org/crawl-data/foo.warc.gz"
        );
    }

    #[test]
    fn capture_key_stable() {
        let c = sample();
        assert_eq!(c.capture_key(), c.capture_key());
    }

    #[test]
    fn challenge_heuristics() {
        let mut c = sample();
        assert!(!c.looks_like_challenge());
        c.status = 403;
        assert!(c.looks_like_challenge());
        c.status = 200;
        c.url = "https://example.com/cdn-cgi/challenge".into();
        assert!(c.looks_like_challenge());
    }
}
