//! Manifests and sampling policy values.

use super::capture::Capture;
use super::query::UrlQuery;
use crate::error::{Error, Result};

/// A resolved, possibly-sampled set of captures for a query.
#[derive(Debug, Clone)]
pub struct Manifest {
    /// The query that produced this manifest.
    pub query: UrlQuery,
    /// The captures retained.
    pub captures: Vec<Capture>,
}

impl Manifest {
    /// Serialize captures as NDJSON (one JSON object per line).
    pub fn to_ndjson(&self) -> Result<String> {
        let mut out = String::new();
        for c in &self.captures {
            out.push_str(&serde_json::to_string(c)?);
            out.push('\n');
        }
        Ok(out)
    }

    /// Parse captures from NDJSON, ignoring blank lines.
    pub fn from_ndjson(query: UrlQuery, s: &str) -> Result<Self> {
        let mut captures = Vec::new();
        for line in s.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            captures.push(serde_json::from_str(line).map_err(|e| Error::Parse(e.to_string()))?);
        }
        Ok(Manifest { query, captures })
    }
}

/// How to sample a per-URL, time-ordered group of captures.
///
/// The *reducer logic* lands in M3 (`model::sampling`); these are the values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sampling {
    /// Keep every capture.
    All,
    /// One per distinct content hash.
    UniqueDigest,
    /// The earliest and latest capture only.
    FirstLast,
    /// First, last, and every digest change (foundry default).
    #[default]
    FirstLastAndDigestChanges,
    /// Adaptive change-point sampling, bounded per URL.
    AdaptiveChangePoint {
        /// Max captures retained per URL.
        max_per_url: u32,
    },
}
