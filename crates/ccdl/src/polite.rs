//! Politeness: per-host rate limiting, concurrency caps, and retry policy.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use governor::clock::DefaultClock;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use tokio::sync::{Mutex, Semaphore};

use crate::error::{Error, Result};

type DirectLimiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

/// Retry policy for transient failures.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum retry attempts.
    pub max_retries: u32,
    /// Base backoff duration.
    pub base: Duration,
    /// Maximum backoff duration.
    pub max: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            max_retries: 5,
            base: Duration::from_millis(500),
            max: Duration::from_secs(30),
        }
    }
}

impl RetryPolicy {
    /// Deterministic exponential backoff (no jitter) for the given attempt,
    /// clamped to `max`. Attempt is 0-indexed.
    #[must_use]
    pub fn backoff(&self, attempt: u32) -> Duration {
        let factor = 2u64.saturating_pow(attempt);
        let d = self
            .base
            .saturating_mul(u32::try_from(factor).unwrap_or(u32::MAX));
        d.min(self.max)
    }
}

/// Politeness configuration and live limiters.
pub struct Politeness {
    max_rps: u32,
    max_concurrency: usize,
    retry: RetryPolicy,
    user_agent: String,
    limiters: Mutex<HashMap<String, Arc<DirectLimiter>>>,
    semaphore: Arc<Semaphore>,
}

impl Politeness {
    /// Build a politeness manager. Rejects placeholder/empty User-Agents.
    pub fn new(
        max_rps: u32,
        max_concurrency: usize,
        retry: RetryPolicy,
        user_agent: impl Into<String>,
    ) -> Result<Self> {
        let user_agent = user_agent.into();
        let trimmed = user_agent.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("user-agent") {
            return Err(Error::Config(
                "a descriptive, non-empty User-Agent is required".into(),
            ));
        }
        let max_rps = max_rps.max(1);
        let max_concurrency = max_concurrency.max(1);
        Ok(Politeness {
            max_rps,
            max_concurrency,
            retry,
            user_agent,
            limiters: Mutex::new(HashMap::new()),
            semaphore: Arc::new(Semaphore::new(max_concurrency)),
        })
    }

    /// The configured User-Agent.
    #[must_use]
    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    /// The retry policy.
    #[must_use]
    pub fn retry(&self) -> &RetryPolicy {
        &self.retry
    }

    /// Maximum concurrency.
    #[must_use]
    pub fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    /// Classify a host into a rate-limit bucket (`index.` vs `data.` vs other).
    fn bucket(host: &str) -> String {
        if host.starts_with("index.") || host.contains("index.commoncrawl") {
            "index".to_owned()
        } else if host.starts_with("data.") {
            "data".to_owned()
        } else {
            host.to_owned()
        }
    }

    async fn limiter_for(&self, host: &str) -> Arc<DirectLimiter> {
        let key = Self::bucket(host);
        let mut map = self.limiters.lock().await;
        map.entry(key)
            .or_insert_with(|| {
                let quota = Quota::per_second(NonZeroU32::new(self.max_rps).unwrap());
                Arc::new(RateLimiter::direct(quota))
            })
            .clone()
    }

    /// Acquire admission to make one request against `host`: waits for a rate
    /// token and a concurrency permit. The returned permit must be held for the
    /// duration of the request.
    pub async fn admit(&self, host: &str) -> Result<tokio::sync::OwnedSemaphorePermit> {
        let limiter = self.limiter_for(host).await;
        limiter.until_ready().await;
        self.semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| Error::Config(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_user_agent() {
        assert!(Politeness::new(1, 1, RetryPolicy::default(), "").is_err());
        assert!(Politeness::new(1, 1, RetryPolicy::default(), "  ").is_err());
        assert!(Politeness::new(1, 1, RetryPolicy::default(), "ccdl/0.1 (me@example.com)").is_ok());
    }

    #[test]
    fn backoff_is_deterministic_and_clamped() {
        let p = RetryPolicy {
            max_retries: 10,
            base: Duration::from_millis(100),
            max: Duration::from_secs(1),
        };
        assert_eq!(p.backoff(0), Duration::from_millis(100));
        assert_eq!(p.backoff(1), Duration::from_millis(200));
        assert_eq!(p.backoff(2), Duration::from_millis(400));
        assert_eq!(p.backoff(20), Duration::from_secs(1));
    }
}
