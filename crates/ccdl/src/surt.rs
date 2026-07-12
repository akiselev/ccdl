//! SURT (Sort-friendly URI Reordering Transform) canonicalization.
//!
//! Matches pywb / Common Crawl `url_surtkey` canonicalization closely enough
//! for dedup (M1) and lexicographic range scans on the columnar index (M3):
//! host reversal, default-port strip, lowercasing, and path normalization.

use url::Url;

/// Compute the SURT key for a URL, e.g.
/// `https://www.Example.com:80/A/b` → `com,example)/a/b`.
///
/// Falls back to a best-effort transform if the input does not parse as an
/// absolute URL (a scheme is assumed).
#[must_use]
pub fn surt(url: &str) -> String {
    let parsed = Url::parse(url).or_else(|_| Url::parse(&format!("http://{url}")));
    let Ok(u) = parsed else {
        return url.to_ascii_lowercase();
    };

    let host = u.host_str().unwrap_or("").to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);

    let mut key = reverse_host(host);
    key.push(')');

    let path = u.path();
    if path.is_empty() || path == "/" {
        key.push('/');
    } else {
        key.push_str(&path.to_ascii_lowercase());
    }

    if let Some(q) = u.query() {
        key.push('?');
        key.push_str(&q.to_ascii_lowercase());
    }
    key
}

fn reverse_host(host: &str) -> String {
    // IP addresses are not reversed.
    if host.parse::<std::net::IpAddr>().is_ok() {
        return host.to_owned();
    }
    let mut parts: Vec<&str> = host.split('.').collect();
    parts.reverse();
    parts.join(",")
}

/// Lexicographic `[start, end)` bounds over `url_surtkey` for a path prefix.
///
/// Used for columnar range pushdown. The end bound increments the last byte of
/// the start key so the half-open range covers exactly the prefix.
#[must_use]
pub fn surt_prefix_bounds(prefix: &str) -> (String, String) {
    let start = surt(prefix);
    let mut end = start.clone().into_bytes();
    // Increment the final byte to form an exclusive upper bound.
    if let Some(last) = end.last_mut() {
        if *last < 0xff {
            *last += 1;
        } else {
            end.push(0);
        }
    }
    (start, String::from_utf8_lossy(&end).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_surt() {
        assert_eq!(
            surt("https://www.example.com/manufacturer/acme"),
            "com,example)/manufacturer/acme"
        );
    }

    #[test]
    fn strips_default_port_and_lowercases() {
        assert_eq!(surt("http://Example.COM:80/A/B"), "com,example)/a/b");
    }

    #[test]
    fn root_path() {
        assert_eq!(surt("https://example.com"), "com,example)/");
    }

    #[test]
    fn bounds_bracket_prefix() {
        let (lo, hi) = surt_prefix_bounds("example.com/manufacturer/");
        assert!(lo < hi);
        assert!(surt("example.com/manufacturer/acme") >= lo);
        assert!(surt("example.com/manufacturer/acme") < hi);
    }
}
