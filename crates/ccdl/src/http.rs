//! HTTP transport: funnels every request through politeness and (for index
//! responses) the cache. Injectable for tests.

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;

use crate::cache::{Cache, CacheKey, EntryMeta};
use crate::error::{Error, Result};
use crate::polite::Politeness;

/// A minimal transport abstraction so tests can inject fixtures.
#[async_trait]
pub trait Transport: Send + Sync {
    /// GET a URL and return the body as text.
    async fn get_text(&self, url: &str) -> Result<String>;
    /// GET a URL and parse the body as JSON.
    async fn get_json(&self, url: &str) -> Result<serde_json::Value>;
    /// GET a byte range from a URL.
    async fn get_range(&self, url: &str, range: &str) -> Result<Bytes>;
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The live `reqwest` transport, polite and cache-backed for index responses.
pub struct HttpTransport {
    client: reqwest::Client,
    polite: Arc<Politeness>,
    cache: Arc<dyn Cache>,
}

impl HttpTransport {
    /// Build a transport over a reqwest client, politeness, and an index cache.
    pub fn new(polite: Arc<Politeness>, cache: Arc<dyn Cache>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(polite.user_agent().to_owned())
            .build()
            .map_err(Error::from)?;
        Ok(HttpTransport {
            client,
            polite,
            cache,
        })
    }

    async fn send_with_retry(&self, req: reqwest::Request) -> Result<reqwest::Response> {
        let host = req.url().host_str().unwrap_or("").to_owned();
        let retry = self.polite.retry().clone();
        let mut attempt = 0;
        loop {
            let permit = self.polite.admit(&host).await?;
            let cloned = req
                .try_clone()
                .ok_or_else(|| Error::Network("request body not cloneable for retry".into()))?;
            let resp = self.client.execute(cloned).await;
            match resp {
                Ok(r) if r.status().is_success() => return Ok(r),
                Ok(r) if r.status().as_u16() == 404 => return Err(Error::NotFound),
                Ok(r) if r.status().as_u16() == 429 || r.status().is_server_error() => {
                    if attempt >= retry.max_retries {
                        if r.status().as_u16() == 429 {
                            return Err(Error::RateLimited { retry_after: None });
                        }
                        return Err(Error::Http {
                            status: r.status().as_u16(),
                        });
                    }
                    drop(permit);
                    tokio::time::sleep(retry.backoff(attempt)).await;
                    attempt += 1;
                }
                Ok(r) => {
                    return Err(Error::Http {
                        status: r.status().as_u16(),
                    });
                }
                Err(e) => {
                    if attempt >= retry.max_retries {
                        return Err(Error::from(e));
                    }
                    drop(permit);
                    tokio::time::sleep(retry.backoff(attempt)).await;
                    attempt += 1;
                }
            }
        }
    }
}

#[async_trait]
impl Transport for HttpTransport {
    async fn get_text(&self, url: &str) -> Result<String> {
        // Cache index/registry responses keyed by URL.
        let key = CacheKey::index(url.to_owned());
        if let Some(hit) = self.cache.get(&key).await? {
            return Ok(String::from_utf8_lossy(&hit.bytes).into_owned());
        }
        let req = self.client.get(url).build().map_err(Error::from)?;
        let resp = self.send_with_retry(req).await?;
        let bytes = resp.bytes().await.map_err(Error::from)?;
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
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn get_json(&self, url: &str) -> Result<serde_json::Value> {
        let text = self.get_text(url).await?;
        serde_json::from_str(&text).map_err(Error::from)
    }

    async fn get_range(&self, url: &str, range: &str) -> Result<Bytes> {
        let _ = host_of(url);
        let req = self
            .client
            .get(url)
            .header(reqwest::header::RANGE, range)
            .build()
            .map_err(Error::from)?;
        let resp = self.send_with_retry(req).await?;
        resp.bytes().await.map_err(Error::from)
    }
}

/// A transport that serves canned responses from an in-memory map, for tests.
#[derive(Default)]
pub struct MockTransport {
    responses: std::collections::HashMap<String, String>,
}

impl MockTransport {
    /// Create an empty mock.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a canned body for an exact URL.
    #[must_use]
    pub fn with(mut self, url: impl Into<String>, body: impl Into<String>) -> Self {
        self.responses.insert(url.into(), body.into());
        self
    }
}

#[async_trait]
impl Transport for MockTransport {
    async fn get_text(&self, url: &str) -> Result<String> {
        self.responses.get(url).cloned().ok_or(Error::NotFound)
    }
    async fn get_json(&self, url: &str) -> Result<serde_json::Value> {
        let text = self.get_text(url).await?;
        serde_json::from_str(&text).map_err(Error::from)
    }
    async fn get_range(&self, url: &str, _range: &str) -> Result<Bytes> {
        self.get_text(url).await.map(Bytes::from)
    }
}
