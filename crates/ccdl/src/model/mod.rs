//! I/O-free, `serde`-friendly core data model.

pub mod capture;
pub mod crawl;
pub mod manifest;
pub mod query;

pub use capture::Capture;
pub use crawl::{CrawlId, CrawlSelector};
pub use manifest::{Manifest, Sampling};
pub use query::{Collapse, Field, Filter, MatchType, StatusPred, UrlQuery};
