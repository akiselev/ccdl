//! `ccdl` — a library for enumerating and downloading captures from
//! [Common Crawl](https://commoncrawl.org).
//!
//! The crate exposes a polite, streaming client (the [`Ccdl`] facade) over
//! Common Crawl's CDX index (and, behind the `table` feature, its columnar
//! index). See `DESIGN.md` and `PLAN.md` for the architecture.
#![deny(missing_docs)]

pub mod error;
pub mod model;
pub mod surt;

#[cfg(feature = "cache-fs")]
pub mod cache;

pub mod polite;

pub mod budget;
pub mod discover;
pub mod http;
pub mod registry;

#[cfg(feature = "index")]
pub mod index;

#[cfg(feature = "warc")]
pub mod warc;

#[cfg(feature = "table")]
pub mod table;

#[cfg(feature = "athena")]
pub mod athena;

pub mod client;

pub use client::Ccdl;
pub use error::{Error, Result};

/// Common imports for typical usage.
pub mod prelude {
    pub use crate::client::Ccdl;
    pub use crate::error::{Error, Result};
    pub use crate::model::capture::Capture;
    pub use crate::model::crawl::{CrawlId, CrawlSelector};
    pub use crate::model::query::{MatchType, UrlQuery};
}
