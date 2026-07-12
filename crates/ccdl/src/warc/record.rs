//! Parse a single WARC record (as sliced from a CC WARC file).

use std::io::Read;

use flate2::read::MultiGzDecoder;

use crate::error::{Error, Result};

/// A parsed WARC record: its WARC headers and the enclosed HTTP block.
#[derive(Debug, Clone)]
pub struct WarcRecord {
    /// WARC header fields (name → value), names lowercased.
    pub headers: Vec<(String, String)>,
    /// The raw enclosed block (usually an HTTP response).
    pub body: Vec<u8>,
}

impl WarcRecord {
    /// Look up a WARC header (case-insensitive).
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }

    /// The `WARC-Type`, e.g. `response`.
    #[must_use]
    pub fn warc_type(&self) -> Option<&str> {
        self.header("warc-type")
    }
}

/// Gunzip a single-record WARC slice and parse it.
///
/// Common Crawl WARC slices are individually gzipped members, so a byte-range
/// slice gunzips standalone.
pub fn parse_warc_slice(gzipped: &[u8]) -> Result<WarcRecord> {
    let mut decoder = MultiGzDecoder::new(gzipped);
    let mut raw = Vec::new();
    decoder
        .read_to_end(&mut raw)
        .map_err(|e| Error::Parse(format!("gunzip failed: {e}")))?;
    parse_warc_bytes(&raw)
}

/// Parse already-decompressed WARC record bytes.
pub fn parse_warc_bytes(raw: &[u8]) -> Result<WarcRecord> {
    // Split WARC header block from the enclosed body at the first blank line.
    let sep =
        find_double_crlf(raw).ok_or_else(|| Error::Parse("no WARC header terminator".into()))?;
    let header_block = &raw[..sep];
    let body = raw[sep + 4..].to_vec();

    let header_str = std::str::from_utf8(header_block)
        .map_err(|_| Error::Parse("WARC headers not utf-8".into()))?;
    let mut lines = header_str.split("\r\n");
    let first = lines.next().unwrap_or("");
    if !first.starts_with("WARC/") {
        return Err(Error::Parse(format!("not a WARC record: {first}")));
    }
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_owned()));
        }
    }
    Ok(WarcRecord { headers, body })
}

fn find_double_crlf(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|w| w == b"\r\n\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    #[test]
    fn parses_record_and_gunzips_standalone() {
        let record = b"WARC/1.0\r\nWARC-Type: response\r\nContent-Length: 5\r\n\r\nHELLO";
        let g = gzip(record);
        let parsed = parse_warc_slice(&g).unwrap();
        assert_eq!(parsed.warc_type(), Some("response"));
        assert_eq!(parsed.body, b"HELLO");
    }
}
