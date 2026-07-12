//! Object-store locations for the cc-index columnar table.

use crate::model::crawl::CrawlId;

/// The HTTPS base for the cc-index table (no AWS creds required).
pub const TABLE_HTTP_BASE: &str = "https://data.commoncrawl.org/cc-index/table/cc-main/warc";

/// The partition directory for a crawl's `subset=warc` Parquet files.
#[must_use]
pub fn partition_prefix(crawl: &CrawlId) -> String {
    format!("{TABLE_HTTP_BASE}/crawl={}/subset=warc/", crawl.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_path() {
        let p = partition_prefix(&CrawlId("CC-MAIN-2026-30".into()));
        assert_eq!(
            p,
            "https://data.commoncrawl.org/cc-index/table/cc-main/warc/crawl=CC-MAIN-2026-30/subset=warc/"
        );
    }
}
