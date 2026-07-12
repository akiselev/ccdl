//! The CDX index backend: query compilation, CDXJ parsing, pagination.

mod cdx_compile;
mod cdx_parse;
mod pagination;

pub use cdx_compile::compile_cdx_url;
pub use cdx_parse::parse_cdxj_line;
pub use pagination::{CaptureStream, Estimate, IndexClient};
