//! Athena adapter (M4, feature `athena`): compile a `UrlQuery` to SQL over the
//! `ccindex` table and (with AWS credentials) submit it, poll, and read the
//! result set. SQL compilation is pure and unit-tested; submission requires an
//! AWS account and is exercised only against a live account.

mod sql;

pub use sql::compile_sql;

/// Configuration for an Athena run.
#[derive(Debug, Clone)]
pub struct AthenaConfig {
    /// The Glue database holding the `ccindex` table.
    pub database: String,
    /// S3 location for Athena query results, e.g. `s3://bucket/prefix/`.
    pub output_location: String,
    /// The `ccindex` table name.
    pub table: String,
}

impl Default for AthenaConfig {
    fn default() -> Self {
        AthenaConfig {
            database: "ccindex".into(),
            output_location: String::new(),
            table: "ccindex".into(),
        }
    }
}
