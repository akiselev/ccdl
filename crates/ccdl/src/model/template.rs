//! URL template parsing and binding, e.g. `host/manufacturer/{name}/{part}`.

use std::collections::BTreeMap;

use crate::error::{Error, Result};

/// A compiled URL template: literal segments interleaved with `{var}` holes.
#[derive(Debug, Clone)]
pub struct UrlTemplate {
    /// The literal prefix up to the first variable (used to seed enumeration).
    prefix: String,
    /// Parsed parts: `Part::Lit` or `Part::Var`.
    parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Lit(String),
    Var(String),
}

impl UrlTemplate {
    /// Parse a template string. `{name}` marks a capture variable.
    pub fn parse(tmpl: &str) -> Result<Self> {
        let mut parts = Vec::new();
        let mut rest = tmpl;
        while let Some(open) = rest.find('{') {
            if open > 0 {
                parts.push(Part::Lit(rest[..open].to_owned()));
            }
            let close = rest[open..]
                .find('}')
                .ok_or_else(|| Error::Config(format!("unclosed template var in {tmpl}")))?
                + open;
            let name = rest[open + 1..close].to_owned();
            if name.is_empty() {
                return Err(Error::Config("empty template var".into()));
            }
            parts.push(Part::Var(name));
            rest = &rest[close + 1..];
        }
        if !rest.is_empty() {
            parts.push(Part::Lit(rest.to_owned()));
        }
        let prefix = match parts.first() {
            Some(Part::Lit(s)) => s.clone(),
            _ => String::new(),
        };
        Ok(UrlTemplate { prefix, parts })
    }

    /// The literal prefix to seed enumeration.
    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Bind a URL against the template, returning captured variables in order.
    ///
    /// Variables are matched greedily up to the next literal separator.
    #[must_use]
    pub fn bind(&self, url: &str) -> Option<BTreeMap<String, String>> {
        let mut vars = BTreeMap::new();
        let mut pos = 0usize;
        let mut i = 0;
        while i < self.parts.len() {
            match &self.parts[i] {
                Part::Lit(lit) => {
                    if url[pos..].starts_with(lit.as_str()) {
                        pos += lit.len();
                    } else {
                        return None;
                    }
                }
                Part::Var(name) => {
                    // Match up to the next literal (or end of string).
                    let next_lit = self.parts.get(i + 1).and_then(|p| match p {
                        Part::Lit(l) => Some(l.as_str()),
                        Part::Var(_) => None,
                    });
                    let remainder = &url[pos..];
                    let end = match next_lit {
                        Some(lit) => remainder.find(lit)?,
                        None => remainder.len(),
                    };
                    if end == 0 {
                        return None;
                    }
                    vars.insert(name.clone(), remainder[..end].to_owned());
                    pos += end;
                }
            }
            i += 1;
        }
        Some(vars)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_prefix() {
        let t = UrlTemplate::parse("example.com/manufacturer/{name}/{part}").unwrap();
        assert_eq!(t.prefix(), "example.com/manufacturer/");
    }

    #[test]
    fn bind_matching() {
        let t = UrlTemplate::parse("example.com/manufacturer/{name}/{part}").unwrap();
        let vars = t.bind("example.com/manufacturer/acme/x1").unwrap();
        assert_eq!(vars.get("name").map(String::as_str), Some("acme"));
        assert_eq!(vars.get("part").map(String::as_str), Some("x1"));
    }

    #[test]
    fn bind_non_matching() {
        let t = UrlTemplate::parse("example.com/manufacturer/{name}/{part}").unwrap();
        assert!(t.bind("other.com/foo/bar/baz").is_none());
    }

    #[test]
    fn unclosed_var_errors() {
        assert!(UrlTemplate::parse("a/{name").is_err());
    }
}
