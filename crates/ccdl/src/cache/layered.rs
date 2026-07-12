//! A two-tier read-through cache.

use async_trait::async_trait;
use bytes::Bytes;

use super::{Cache, CacheHit, CacheKey, CacheStats, EntryMeta};
use crate::error::Result;

/// Reads hit `fast` first, falling back to `slow` and back-filling `fast`.
/// Writes go to both tiers.
pub struct LayeredCache<F, S> {
    fast: F,
    slow: S,
}

impl<F, S> LayeredCache<F, S> {
    /// Compose two caches into a read-through layer.
    pub fn new(fast: F, slow: S) -> Self {
        LayeredCache { fast, slow }
    }
}

#[async_trait]
impl<F: Cache, S: Cache> Cache for LayeredCache<F, S> {
    async fn get(&self, key: &CacheKey) -> Result<Option<CacheHit>> {
        if let Some(hit) = self.fast.get(key).await? {
            return Ok(Some(hit));
        }
        if let Some(hit) = self.slow.get(key).await? {
            self.fast
                .put(
                    key,
                    hit.bytes.clone(),
                    EntryMeta {
                        size: hit.bytes.len() as u64,
                        ttl: None,
                    },
                )
                .await?;
            return Ok(Some(hit));
        }
        Ok(None)
    }

    async fn put(&self, key: &CacheKey, bytes: Bytes, meta: EntryMeta) -> Result<()> {
        self.fast.put(key, bytes.clone(), meta).await?;
        self.slow.put(key, bytes, meta).await
    }

    async fn stats(&self) -> Result<CacheStats> {
        self.slow.stats().await
    }

    async fn gc(&self) -> Result<u64> {
        let a = self.fast.gc().await?;
        let b = self.slow.gc().await?;
        Ok(a + b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{FsCache, NoopCache};

    #[tokio::test]
    async fn read_through() {
        let dir = tempfile::tempdir().unwrap();
        let slow = FsCache::new(dir.path()).unwrap();
        let layered = LayeredCache::new(NoopCache, slow);
        let key = CacheKey::index("x");
        layered
            .put(
                &key,
                Bytes::from_static(b"v"),
                EntryMeta { size: 1, ttl: None },
            )
            .await
            .unwrap();
        assert!(layered.get(&key).await.unwrap().is_some());
    }
}
