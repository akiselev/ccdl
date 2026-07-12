//! Sampling reducers over per-URL, time-ordered capture groups.

use std::collections::HashMap;

use super::capture::Capture;
use super::manifest::Sampling;

/// Apply a sampling policy to a flat list of captures.
///
/// Captures are grouped by `urlkey`, each group sorted by timestamp ascending,
/// and the policy's reducer applied. The retained captures are returned in
/// `(urlkey, timestamp)` order.
#[must_use]
pub fn sample(captures: Vec<Capture>, policy: Sampling) -> Vec<Capture> {
    let mut groups: HashMap<String, Vec<Capture>> = HashMap::new();
    for c in captures {
        groups.entry(c.urlkey.clone()).or_default().push(c);
    }
    let mut keys: Vec<String> = groups.keys().cloned().collect();
    keys.sort();

    let mut out = Vec::new();
    for key in keys {
        let mut group = groups.remove(&key).unwrap();
        group.sort_by_key(|c| c.timestamp);
        out.extend(reduce(group, policy));
    }
    out
}

fn reduce(group: Vec<Capture>, policy: Sampling) -> Vec<Capture> {
    match policy {
        Sampling::All => group,
        Sampling::UniqueDigest => {
            let mut seen = std::collections::HashSet::new();
            group
                .into_iter()
                .filter(|c| seen.insert(c.digest.clone()))
                .collect()
        }
        Sampling::FirstLast => first_last(group),
        Sampling::FirstLastAndDigestChanges => first_last_digest_changes(group),
        Sampling::AdaptiveChangePoint { max_per_url } => {
            let changed = first_last_digest_changes(group);
            cap_len(changed, max_per_url as usize)
        }
    }
}

fn first_last(group: Vec<Capture>) -> Vec<Capture> {
    let len = group.len();
    if len <= 2 {
        return group;
    }
    let mut it = group.into_iter();
    let first = it.next().unwrap();
    let last = it.last().unwrap();
    vec![first, last]
}

fn first_last_digest_changes(group: Vec<Capture>) -> Vec<Capture> {
    if group.len() <= 1 {
        return group;
    }
    let last_idx = group.len() - 1;
    let mut out = Vec::new();
    let mut prev_digest: Option<String> = None;
    for (i, c) in group.into_iter().enumerate() {
        let is_edge = i == 0 || i == last_idx;
        let changed = prev_digest.as_ref() != Some(&c.digest);
        if is_edge || changed {
            prev_digest = Some(c.digest.clone());
            out.push(c);
        } else {
            prev_digest = Some(c.digest.clone());
        }
    }
    out
}

fn cap_len(mut group: Vec<Capture>, max: usize) -> Vec<Capture> {
    if max == 0 || group.len() <= max {
        return group;
    }
    // Keep first and last, then evenly-spaced middles.
    let mut kept = Vec::with_capacity(max);
    let last = group.len() - 1;
    for i in 0..max {
        let idx = i * last / (max - 1);
        kept.push(idx);
    }
    kept.dedup();
    let mut out = Vec::new();
    for (i, c) in group.drain(..).enumerate() {
        if kept.contains(&i) {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::crawl::CrawlId;
    use chrono::{TimeZone, Utc};

    fn cap(url: &str, secs: i64, digest: &str) -> Capture {
        Capture {
            crawl: CrawlId("C".into()),
            urlkey: url.into(),
            url: url.into(),
            timestamp: Utc.timestamp_opt(secs, 0).unwrap(),
            status: 200,
            mime: None,
            mime_detected: None,
            languages: vec![],
            digest: digest.into(),
            length: 1,
            offset: 0,
            filename: "f".into(),
            redirect: None,
            truncated: None,
        }
    }

    #[test]
    fn unique_digest() {
        let caps = vec![cap("a", 1, "x"), cap("a", 2, "x"), cap("a", 3, "y")];
        let out = sample(caps, Sampling::UniqueDigest);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn first_last_only() {
        let caps = vec![cap("a", 1, "x"), cap("a", 2, "y"), cap("a", 3, "z")];
        let out = sample(caps, Sampling::FirstLast);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].timestamp.timestamp(), 1);
        assert_eq!(out[1].timestamp.timestamp(), 3);
    }

    #[test]
    fn first_last_digest_changes() {
        // x x y y z -> keep first(x), y (change), z(change/last) = 3
        let caps = vec![
            cap("a", 1, "x"),
            cap("a", 2, "x"),
            cap("a", 3, "y"),
            cap("a", 4, "y"),
            cap("a", 5, "z"),
        ];
        let out = sample(caps, Sampling::FirstLastAndDigestChanges);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn adaptive_caps_length() {
        let caps: Vec<_> = (0..10).map(|i| cap("a", i, &format!("d{i}"))).collect();
        let out = sample(caps, Sampling::AdaptiveChangePoint { max_per_url: 3 });
        assert!(out.len() <= 3);
    }
}
