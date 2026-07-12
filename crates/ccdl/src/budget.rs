//! Run budgets: cap a stream by record count or bytes, with a resume token.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::StreamExt;

use crate::error::Error;
use crate::index::CaptureStream;

/// A budget over a capture stream.
#[derive(Debug, Clone, Copy, Default)]
pub struct Budget {
    /// Maximum records (captures) to emit.
    pub max_records: Option<u64>,
    /// Maximum total `length` bytes across emitted captures.
    pub max_bytes: Option<u64>,
    /// Maximum index pages (informational; enforced by the index client).
    pub max_pages: Option<u64>,
}

impl Budget {
    /// True if no limits are set.
    #[must_use]
    pub fn is_unlimited(&self) -> bool {
        self.max_records.is_none() && self.max_bytes.is_none()
    }

    /// Wrap a stream so it errors with [`Error::BudgetExhausted`] once a limit
    /// is crossed. Captures emitted before the limit pass through unchanged; the
    /// resume token is the last successfully-emitted capture's key.
    #[must_use]
    pub fn apply(self, stream: CaptureStream) -> CaptureStream {
        if self.is_unlimited() {
            return stream;
        }
        let records = Arc::new(AtomicU64::new(0));
        let bytes = Arc::new(AtomicU64::new(0));
        let last = Arc::new(std::sync::Mutex::new(String::new()));
        let out = stream.map(move |item| {
            let cap = item?;
            if let Some(max) = self.max_records {
                if records.load(Ordering::Relaxed) >= max {
                    return Err(Error::BudgetExhausted {
                        resume: last.lock().unwrap().clone(),
                    });
                }
            }
            if let Some(max) = self.max_bytes {
                if bytes.load(Ordering::Relaxed) + cap.length > max {
                    return Err(Error::BudgetExhausted {
                        resume: last.lock().unwrap().clone(),
                    });
                }
            }
            records.fetch_add(1, Ordering::Relaxed);
            bytes.fetch_add(cap.length, Ordering::Relaxed);
            *last.lock().unwrap() = cap.capture_key();
            Ok(cap)
        });
        Box::pin(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::capture::Capture;
    use crate::model::crawl::CrawlId;
    use chrono::Utc;

    fn cap(i: u64) -> Capture {
        Capture {
            crawl: CrawlId("C".into()),
            urlkey: format!("k{i}"),
            url: format!("https://x/{i}"),
            timestamp: Utc::now(),
            status: 200,
            mime: None,
            mime_detected: None,
            languages: vec![],
            digest: format!("d{i}"),
            length: 10,
            offset: i,
            filename: "f".into(),
            redirect: None,
            truncated: None,
        }
    }

    #[tokio::test]
    async fn caps_by_records() {
        let s: CaptureStream = Box::pin(futures::stream::iter((0..5).map(|i| Ok(cap(i)))));
        let budget = Budget {
            max_records: Some(2),
            ..Default::default()
        };
        let out: Vec<_> = budget.apply(s).collect().await;
        // 2 ok, then a BudgetExhausted error.
        assert_eq!(out.iter().filter(|r| r.is_ok()).count(), 2);
        assert!(matches!(
            out.last(),
            Some(Err(Error::BudgetExhausted { .. }))
        ));
    }
}
