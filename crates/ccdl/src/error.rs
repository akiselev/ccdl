//! Error and result types for `ccdl`.
//!
//! Note (DESIGN §6.5): an *empty* enumeration is `Ok` with zero rows, never an
//! error.

use std::time::Duration;

/// The crate-wide result type.
pub type Result<T> = std::result::Result<T, Error>;

/// All errors surfaced by `ccdl`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The server asked us to slow down; `retry_after` is honored if present.
    #[error("rate limited (retry after {retry_after:?})")]
    RateLimited {
        /// Suggested wait before retrying, from `Retry-After` if provided.
        retry_after: Option<Duration>,
    },

    /// A requested resource does not exist.
    #[error("not found")]
    NotFound,

    /// An HTTP request returned an unexpected status.
    #[error("unexpected http status {status}")]
    Http {
        /// The HTTP status code.
        status: u16,
    },

    /// The index result spans more pages than the configured guard allows.
    #[error("index too large: {pages} pages")]
    IndexTooLarge {
        /// Number of pages the query would require.
        pages: u64,
    },

    /// A parse failure (CDXJ, WARC, HTTP, Parquet, …).
    #[error("parse error: {0}")]
    Parse(String),

    /// A cache-layer failure.
    #[error("cache error: {0}")]
    Cache(String),

    /// Invalid configuration (e.g. placeholder User-Agent).
    #[error("config error: {0}")]
    Config(String),

    /// A backend-specific failure (columnar, Athena, …).
    #[error("backend error: {0}")]
    Backend(String),

    /// An I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// A network/transport error.
    #[error("network error: {0}")]
    Network(String),

    /// A run hit its configured budget; carries a resume token for the next run.
    #[error("budget exhausted (resume: {resume})")]
    BudgetExhausted {
        /// A token identifying where to resume (last emitted capture key).
        resume: String,
    },
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        if let Some(status) = e.status() {
            Error::Http {
                status: status.as_u16(),
            }
        } else {
            Error::Network(e.to_string())
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Parse(e.to_string())
    }
}
