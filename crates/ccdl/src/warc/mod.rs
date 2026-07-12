//! WARC fetch and parse (M2): range-GET a capture's record, gunzip, parse the
//! WARC headers and enclosed HTTP response, with payload content-addressing.

mod fetch;
pub mod http;
mod record;

pub use fetch::WarcClient;
pub use http::HttpCapture;
pub use record::{WarcRecord, parse_warc_bytes, parse_warc_slice};
