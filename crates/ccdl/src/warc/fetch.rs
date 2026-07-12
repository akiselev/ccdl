//! Fetch a capture's WARC record via a byte-range GET, with payload CAS.

use std::sync::Arc;

use bytes::Bytes;

use super::http::HttpCapture;
use super::record::{WarcRecord, parse_warc_slice};
use crate::cache::{Cache, CacheKey, EntryMeta};
use crate::error::Result;
use crate::http::Transport;
use crate::model::capture::Capture;

/// Fetches WARC records, caching payloads content-addressed by digest.
pub struct WarcClient {
    transport: Arc<dyn Transport>,
    cache: Arc<dyn Cache>,
}

impl WarcClient {
    /// Create a WARC client.
    #[must_use]
    pub fn new(transport: Arc<dyn Transport>, cache: Arc<dyn Cache>) -> Self {
        WarcClient { transport, cache }
    }

    /// Fetch the raw (gzipped) WARC slice for a capture, using the payload
    /// cache (keyed by `content_digest`) to skip the network on a hit.
    pub async fn fetch_raw(&self, capture: &Capture) -> Result<Bytes> {
        let key = CacheKey::payload(capture.digest.clone());
        if let Some(hit) = self.cache.get(&key).await? {
            return Ok(hit.bytes);
        }
        let bytes = self
            .transport
            .get_range(&capture.warc_url(), &capture.range_header())
            .await?;
        self.cache
            .put(
                &key,
                bytes.clone(),
                EntryMeta {
                    size: bytes.len() as u64,
                    ttl: None,
                },
            )
            .await?;
        Ok(bytes)
    }

    /// Fetch and parse a capture's WARC record.
    pub async fn fetch(&self, capture: &Capture) -> Result<WarcRecord> {
        let raw = self.fetch_raw(capture).await?;
        parse_warc_slice(&raw)
    }

    /// Fetch a capture and parse the enclosed HTTP response.
    pub async fn fetch_http(&self, capture: &Capture) -> Result<HttpCapture> {
        let record = self.fetch(capture).await?;
        HttpCapture::parse(&record.body)
    }
}
