//! In-memory and no-op cache implementations.

use async_trait::async_trait;
use bytes::Bytes;

use super::{Cache, CacheHit, CacheKey, CacheStats, EntryMeta};
use crate::error::Result;

/// A cache that stores nothing.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopCache;

#[async_trait]
impl Cache for NoopCache {
    async fn get(&self, _key: &CacheKey) -> Result<Option<CacheHit>> {
        Ok(None)
    }
    async fn put(&self, _key: &CacheKey, _bytes: Bytes, _meta: EntryMeta) -> Result<()> {
        Ok(())
    }
    async fn stats(&self) -> Result<CacheStats> {
        Ok(CacheStats::default())
    }
    async fn gc(&self) -> Result<u64> {
        Ok(0)
    }
}

/// A bounded in-memory cache backed by `moka`.
#[cfg(feature = "moka")]
pub struct MemoryCache {
    inner: moka::future::Cache<String, Bytes>,
}

#[cfg(feature = "moka")]
impl MemoryCache {
    /// Create a cache holding up to `max_capacity` bytes.
    #[must_use]
    pub fn new(max_capacity: u64) -> Self {
        MemoryCache {
            inner: moka::future::Cache::builder()
                .max_capacity(max_capacity)
                .weigher(|_k, v: &Bytes| u32::try_from(v.len()).unwrap_or(u32::MAX))
                .build(),
        }
    }
}

#[cfg(feature = "moka")]
#[async_trait]
impl Cache for MemoryCache {
    async fn get(&self, key: &CacheKey) -> Result<Option<CacheHit>> {
        Ok(self
            .inner
            .get(&key.flat())
            .await
            .map(|bytes| CacheHit { bytes }))
    }
    async fn put(&self, key: &CacheKey, bytes: Bytes, _meta: EntryMeta) -> Result<()> {
        self.inner.insert(key.flat(), bytes).await;
        Ok(())
    }
    async fn stats(&self) -> Result<CacheStats> {
        Ok(CacheStats {
            entries: self.inner.entry_count(),
            bytes: self.inner.weighted_size(),
        })
    }
    async fn gc(&self) -> Result<u64> {
        self.inner.run_pending_tasks().await;
        Ok(0)
    }
}
