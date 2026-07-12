//! Parse the HTTP response enclosed in a WARC `response` record.

use std::io::Read;

use crate::error::{Error, Result};

/// A parsed HTTP response captured inside a WARC record.
#[derive(Debug, Clone)]
pub struct HttpCapture {
    /// HTTP status code.
    pub status: u16,
    /// Response headers (name lowercased → value).
    pub headers: Vec<(String, String)>,
    /// Raw (still content-encoded) response body.
    pub body: Vec<u8>,
}

impl HttpCapture {
    /// Look up a header (case-insensitive).
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Parse an HTTP response block (status line + headers + body).
    pub fn parse(block: &[u8]) -> Result<Self> {
        let mut header_buf = [httparse::EMPTY_HEADER; 128];
        let mut resp = httparse::Response::new(&mut header_buf);
        let status_res = resp
            .parse(block)
            .map_err(|e| Error::Parse(format!("http parse: {e}")))?;
        let body_start = match status_res {
            httparse::Status::Complete(n) => n,
            httparse::Status::Partial => {
                return Err(Error::Parse("incomplete HTTP header block".into()));
            }
        };
        let status = resp
            .code
            .ok_or_else(|| Error::Parse("no status code".into()))?;
        let headers = resp
            .headers
            .iter()
            .map(|h| {
                (
                    h.name.to_ascii_lowercase(),
                    String::from_utf8_lossy(h.value).into_owned(),
                )
            })
            .collect();
        Ok(HttpCapture {
            status,
            headers,
            body: block[body_start..].to_vec(),
        })
    }

    /// Decode the body: apply `content-encoding` (gzip) and lossily decode as
    /// UTF-8 (charset handling is best-effort).
    pub fn text(&self) -> Result<String> {
        let decoded = self.decoded_body()?;
        Ok(String::from_utf8_lossy(&decoded).into_owned())
    }

    /// The body with any `content-encoding` removed.
    pub fn decoded_body(&self) -> Result<Vec<u8>> {
        let enc = self
            .header("content-encoding")
            .unwrap_or("")
            .to_ascii_lowercase();
        if enc.contains("gzip") {
            let mut d = flate2::read::MultiGzDecoder::new(&self.body[..]);
            let mut out = Vec::new();
            d.read_to_end(&mut out)
                .map_err(|e| Error::Parse(format!("gzip body: {e}")))?;
            Ok(out)
        } else {
            Ok(self.body.clone())
        }
    }

    /// Heuristic: does the response look like a bot-challenge page?
    #[must_use]
    pub fn looks_like_challenge(&self) -> bool {
        if matches!(self.status, 403 | 429 | 503) {
            return true;
        }
        if let Some(server) = self.header("server") {
            if server.eq_ignore_ascii_case("cloudflare") && self.status != 200 {
                return true;
            }
        }
        let snippet = String::from_utf8_lossy(&self.body[..self.body.len().min(4096)]);
        let s = snippet.to_ascii_lowercase();
        s.contains("just a moment") || s.contains("cf-challenge") || s.contains("captcha")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_200() {
        let block = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html>hi</html>";
        let cap = HttpCapture::parse(block).unwrap();
        assert_eq!(cap.status, 200);
        assert_eq!(cap.header("content-type"), Some("text/html"));
        assert_eq!(cap.text().unwrap(), "<html>hi</html>");
    }

    #[test]
    fn parses_redirect() {
        let block = b"HTTP/1.1 301 Moved\r\nLocation: https://example.com/new\r\n\r\n";
        let cap = HttpCapture::parse(block).unwrap();
        assert_eq!(cap.status, 301);
        assert_eq!(cap.header("location"), Some("https://example.com/new"));
    }

    #[test]
    fn detects_challenge() {
        let block = b"HTTP/1.1 403 Forbidden\r\nServer: cloudflare\r\n\r\nJust a moment...";
        let cap = HttpCapture::parse(block).unwrap();
        assert!(cap.looks_like_challenge());
    }
}
