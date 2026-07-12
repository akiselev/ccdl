//! Pluggable cache layer with three domains: `registry:`, `index:`, `payload:`.

mod fs;
mod layered;
mod memory;

pub use fs::FsCache;
pub use layered::LayeredCache;
pub use memory::NoopCache;

#[cfg(feature = "moka")]
pub use memory::MemoryCache;

use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;

use crate::error::Result;

/// A namespaced cache key. `Display` renders `domain:key`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    /// The domain namespace (`registry`, `index`, `payload`).
    pub domain: &'static str,
    /// The key within the domain.
    pub key: String,
}

impl CacheKey {
    /// A `registry:` key.
    #[must_use]
    pub fn registry(key: impl Into<String>) -> Self {
        CacheKey {
            domain: "registry",
            key: key.into(),
        }
    }
    /// An `index:` key.
    #[must_use]
    pub fn index(key: impl Into<String>) -> Self {
        CacheKey {
            domain: "index",
            key: key.into(),
        }
    }
    /// A `payload:` key (keyed by content digest).
    #[must_use]
    pub fn payload(key: impl Into<String>) -> Self {
        CacheKey {
            domain: "payload",
            key: key.into(),
        }
    }

    /// The flat string form used for on-disk naming.
    #[must_use]
    pub fn flat(&self) -> String {
        format!("{}:{}", self.domain, self.key)
    }
}

/// Metadata stored alongside a cached entry.
#[derive(Debug, Clone, Copy)]
pub struct EntryMeta {
    /// Stored byte length.
    pub size: u64,
    /// Optional TTL from time of write.
    pub ttl: Option<Duration>,
}

/// A cache hit: bytes plus metadata.
#[derive(Debug, Clone)]
pub struct CacheHit {
    /// The stored bytes.
    pub bytes: Bytes,
}

/// Cache statistics.
#[derive(Debug, Clone, Copy, Default)]
pub struct CacheStats {
    /// Number of entries.
    pub entries: u64,
    /// Total bytes stored.
    pub bytes: u64,
}

/// The async cache trait.
#[async_trait]
pub trait Cache: Send + Sync {
    /// Fetch an entry, if present and unexpired.
    async fn get(&self, key: &CacheKey) -> Result<Option<CacheHit>>;
    /// Store an entry.
    async fn put(&self, key: &CacheKey, bytes: Bytes, meta: EntryMeta) -> Result<()>;
    /// Aggregate statistics.
    async fn stats(&self) -> Result<CacheStats>;
    /// Garbage-collect expired entries; returns bytes reclaimed.
    async fn gc(&self) -> Result<u64>;
}
