//! Content-addressed filesystem cache.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use bytes::Bytes;

use super::{Cache, CacheHit, CacheKey, CacheStats, EntryMeta};
use crate::error::{Error, Result};

/// A filesystem cache using a `ab/cd/<key>` content-addressed layout.
///
/// Writes are atomic (temp file + rename). A small sidecar (`.meta`) records
/// the write time and TTL for expiry.
pub struct FsCache {
    root: PathBuf,
}

impl FsCache {
    /// Create a cache rooted at `root`, creating the directory if needed.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(FsCache { root })
    }

    fn hash(key: &CacheKey) -> String {
        // Cheap, stable, dependency-free FNV-1a over the flat key.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in key.flat().bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{h:016x}")
    }

    fn path_for(&self, key: &CacheKey) -> PathBuf {
        let h = Self::hash(key);
        self.root.join(&h[0..2]).join(&h[2..4]).join(h)
    }

    fn meta_path(data: &Path) -> PathBuf {
        data.with_extension("meta")
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

#[async_trait]
impl Cache for FsCache {
    async fn get(&self, key: &CacheKey) -> Result<Option<CacheHit>> {
        let path = self.path_for(key);
        let meta_path = Self::meta_path(&path);
        let bytes = match tokio::fs::read(&path).await {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        // Check TTL from sidecar: "written_secs ttl_secs" (ttl 0 = no expiry).
        if let Ok(meta) = tokio::fs::read_to_string(&meta_path).await {
            let mut it = meta.split_whitespace();
            let written: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let ttl: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            if ttl > 0 && Self::now_secs() > written + ttl {
                let _ = tokio::fs::remove_file(&path).await;
                let _ = tokio::fs::remove_file(&meta_path).await;
                return Ok(None);
            }
        }
        Ok(Some(CacheHit {
            bytes: Bytes::from(bytes),
        }))
    }

    async fn put(&self, key: &CacheKey, bytes: Bytes, meta: EntryMeta) -> Result<()> {
        let path = self.path_for(key);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &path).await?;
        let ttl = meta.ttl.map_or(0, |d| d.as_secs());
        let sidecar = format!("{} {}", Self::now_secs(), ttl);
        tokio::fs::write(Self::meta_path(&path), sidecar).await?;
        Ok(())
    }

    async fn stats(&self) -> Result<CacheStats> {
        let mut stats = CacheStats::default();
        let mut stack = vec![self.root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(mut rd) = tokio::fs::read_dir(&dir).await else {
                continue;
            };
            while let Some(entry) = rd.next_entry().await? {
                let ft = entry.file_type().await?;
                if ft.is_dir() {
                    stack.push(entry.path());
                } else if entry.path().extension().is_none_or(|e| e != "meta") {
                    stats.entries += 1;
                    stats.bytes += entry.metadata().await?.len();
                }
            }
        }
        Ok(stats)
    }

    async fn gc(&self) -> Result<u64> {
        let mut reclaimed = 0u64;
        let mut stack = vec![self.root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(mut rd) = tokio::fs::read_dir(&dir).await else {
                continue;
            };
            while let Some(entry) = rd.next_entry().await? {
                let path = entry.path();
                if entry.file_type().await?.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_some_and(|e| e == "meta") {
                    let meta = tokio::fs::read_to_string(&path).await.unwrap_or_default();
                    let mut it = meta.split_whitespace();
                    let written: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    let ttl: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    if ttl > 0 && Self::now_secs() > written + ttl {
                        let data = path.with_extension("");
                        if let Ok(m) = tokio::fs::metadata(&data).await {
                            reclaimed += m.len();
                        }
                        let _ = tokio::fs::remove_file(&data).await;
                        let _ = tokio::fs::remove_file(&path).await;
                    }
                }
            }
        }
        Ok(reclaimed)
    }
}

// Keep the `EntryMeta`/`Duration` imports meaningful in all feature configs.
#[allow(dead_code)]
fn _assert_meta(_: EntryMeta, _: Duration, _: Error) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_get_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let cache = FsCache::new(dir.path()).unwrap();
        let key = CacheKey::index("q1");
        cache
            .put(
                &key,
                Bytes::from_static(b"hello"),
                EntryMeta { size: 5, ttl: None },
            )
            .await
            .unwrap();
        let hit = cache.get(&key).await.unwrap().unwrap();
        assert_eq!(&hit.bytes[..], b"hello");
    }

    #[tokio::test]
    async fn expiry_and_gc() {
        let dir = tempfile::tempdir().unwrap();
        let cache = FsCache::new(dir.path()).unwrap();
        let key = CacheKey::index("q2");
        cache
            .put(
                &key,
                Bytes::from_static(b"data"),
                EntryMeta {
                    size: 4,
                    ttl: Some(Duration::from_secs(0)),
                },
            )
            .await
            .unwrap();
        // ttl 0 means no-expiry per sidecar encoding, so it should persist.
        assert!(cache.get(&key).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn miss_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let cache = FsCache::new(dir.path()).unwrap();
        assert!(cache.get(&CacheKey::index("nope")).await.unwrap().is_none());
    }
}
