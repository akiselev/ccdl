//! Sitemap seed discovery: read captured `robots.txt` / `sitemap.xml` from
//! Common Crawl to seed enumeration of unknown URL schemes.

/// Extract `<loc>` URLs from a `sitemap.xml` (or sitemap-index) body.
#[must_use]
pub fn parse_sitemap(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(open) = rest.find("<loc>") {
        let after = &rest[open + 5..];
        if let Some(close) = after.find("</loc>") {
            let url = after[..close].trim();
            if !url.is_empty() {
                out.push(decode_entities(url));
            }
            rest = &after[close + 6..];
        } else {
            break;
        }
    }
    out
}

/// Extract `Sitemap:` directives from a `robots.txt` body.
#[must_use]
pub fn parse_robots_sitemaps(robots: &str) -> Vec<String> {
    robots
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            let lower = l.to_ascii_lowercase();
            lower
                .strip_prefix("sitemap:")
                .map(|_| l[l.find(':').unwrap() + 1..].trim().to_owned())
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sitemap_locs() {
        let xml = "<urlset><url><loc>https://x.com/a</loc></url><url><loc>https://x.com/b?p=1&amp;q=2</loc></url></urlset>";
        let urls = parse_sitemap(xml);
        assert_eq!(urls, vec!["https://x.com/a", "https://x.com/b?p=1&q=2"]);
    }

    #[test]
    fn parses_robots_sitemaps() {
        let robots = "User-agent: *\nDisallow: /x\nSitemap: https://x.com/sitemap.xml\n";
        assert_eq!(
            parse_robots_sitemaps(robots),
            vec!["https://x.com/sitemap.xml"]
        );
    }
}
