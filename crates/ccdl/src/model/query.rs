//! The backend-agnostic query model.

use chrono::{DateTime, Utc};

use super::crawl::{CrawlId, CrawlSelector};

/// How the `pattern` matches against index URLs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchType {
    /// Exact URL match.
    Exact,
    /// Prefix (path) match.
    Prefix,
    /// All URLs on a single host.
    Host,
    /// All URLs on a host and its subdomains.
    Domain,
}

impl MatchType {
    /// The CDX `matchType` token.
    #[must_use]
    pub fn cdx_token(self) -> &'static str {
        match self {
            MatchType::Exact => "exact",
            MatchType::Prefix => "prefix",
            MatchType::Host => "host",
            MatchType::Domain => "domain",
        }
    }
}

/// A predicate on `fetch_status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusPred {
    /// Exactly this status.
    Eq(u16),
    /// Any of these statuses.
    In(Vec<u16>),
    /// A status class, e.g. `2` for 2xx.
    Class(u8),
}

/// A single query filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    /// Filter on status.
    Status(StatusPred),
    /// Filter on declared or detected MIME.
    Mime(String),
    /// Filter on detected content language.
    Language(String),
    /// Filter on the URL TLD.
    Tld(String),
    /// Filter with a URL regular expression.
    UrlRegex(String),
}

/// Collapse (dedup) directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Collapse {
    /// Collapse on content digest.
    Digest,
    /// Collapse on url key.
    UrlKey,
    /// Collapse on the first `n` chars of the timestamp.
    Timestamp(u8),
}

impl Collapse {
    /// The CDX `collapse` field token.
    #[must_use]
    pub fn cdx_token(&self) -> String {
        match self {
            Collapse::Digest => "digest".to_owned(),
            Collapse::UrlKey => "urlkey".to_owned(),
            Collapse::Timestamp(n) => format!("timestamp:{n}"),
        }
    }
}

/// A projectable index field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Field {
    UrlKey,
    Url,
    Timestamp,
    Status,
    Mime,
    MimeDetected,
    Digest,
    Length,
    Offset,
    Filename,
    Languages,
    Redirect,
}

impl Field {
    /// The CDX field token.
    #[must_use]
    pub fn cdx_token(self) -> &'static str {
        match self {
            Field::UrlKey => "urlkey",
            Field::Url => "url",
            Field::Timestamp => "timestamp",
            Field::Status => "status",
            Field::Mime => "mime",
            Field::MimeDetected => "mime-detected",
            Field::Digest => "digest",
            Field::Length => "length",
            Field::Offset => "offset",
            Field::Filename => "filename",
            Field::Languages => "languages",
            Field::Redirect => "redirect",
        }
    }
}

/// The single query description both backends compile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlQuery {
    /// The pattern, e.g. `example.com/manufacturer/`.
    pub pattern: String,
    /// How the pattern matches.
    pub match_type: MatchType,
    /// Which crawls to span.
    pub crawls: CrawlSelector,
    /// Filters applied server-side where possible.
    pub filters: Vec<Filter>,
    /// Optional collapse directive.
    pub collapse: Option<Collapse>,
    /// Optional field projection.
    pub fields: Option<Vec<Field>>,
    /// Optional inclusive time range.
    pub time_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    /// Optional row limit.
    pub limit: Option<usize>,
}

impl UrlQuery {
    fn new(pattern: impl Into<String>, match_type: MatchType) -> Self {
        UrlQuery {
            pattern: pattern.into(),
            match_type,
            crawls: CrawlSelector::default(),
            filters: Vec::new(),
            collapse: None,
            fields: None,
            time_range: None,
            limit: None,
        }
    }

    /// A prefix query.
    #[must_use]
    pub fn prefix(pattern: impl Into<String>) -> Self {
        Self::new(pattern, MatchType::Prefix)
    }

    /// A host query.
    #[must_use]
    pub fn host(pattern: impl Into<String>) -> Self {
        Self::new(pattern, MatchType::Host)
    }

    /// A domain query.
    #[must_use]
    pub fn domain(pattern: impl Into<String>) -> Self {
        Self::new(pattern, MatchType::Domain)
    }

    /// An exact query.
    #[must_use]
    pub fn exact(pattern: impl Into<String>) -> Self {
        Self::new(pattern, MatchType::Exact)
    }

    /// Require an exact status.
    #[must_use]
    pub fn status(mut self, status: u16) -> Self {
        self.filters.push(Filter::Status(StatusPred::Eq(status)));
        self
    }

    /// Require a status class (e.g. 2 for 2xx).
    #[must_use]
    pub fn status_class(mut self, class: u8) -> Self {
        self.filters.push(Filter::Status(StatusPred::Class(class)));
        self
    }

    /// Filter on MIME type.
    #[must_use]
    pub fn mime(mut self, mime: impl Into<String>) -> Self {
        self.filters.push(Filter::Mime(mime.into()));
        self
    }

    /// Set the collapse directive.
    #[must_use]
    pub fn collapse(mut self, collapse: Collapse) -> Self {
        self.collapse = Some(collapse);
        self
    }

    /// Set the crawl selector.
    #[must_use]
    pub fn crawls(mut self, crawls: CrawlSelector) -> Self {
        self.crawls = crawls;
        self
    }

    /// Restrict to explicit crawl ids.
    #[must_use]
    pub fn crawl_ids(mut self, ids: Vec<CrawlId>) -> Self {
        self.crawls = CrawlSelector::Ids(ids);
        self
    }

    /// Set the row limit.
    #[must_use]
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Set the inclusive time range.
    #[must_use]
    pub fn time_range(mut self, from: DateTime<Utc>, to: DateTime<Utc>) -> Self {
        self.time_range = Some((from, to));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builders() {
        let q = UrlQuery::prefix("example.com/manufacturer/")
            .status(200)
            .mime("text/html")
            .collapse(Collapse::Digest)
            .limit(10);
        assert_eq!(q.match_type, MatchType::Prefix);
        assert_eq!(q.filters.len(), 2);
        assert_eq!(q.collapse, Some(Collapse::Digest));
        assert_eq!(q.limit, Some(10));
    }

    #[test]
    fn collapse_tokens() {
        assert_eq!(Collapse::Timestamp(10).cdx_token(), "timestamp:10");
    }
}
