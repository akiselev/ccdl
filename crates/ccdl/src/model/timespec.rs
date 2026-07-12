//! Flexible date/time parsing for crawl selection and time-range filters.
//!
//! Accepts absolute forms (`2021`, `2021-06`, `2021-06-15`, RFC3339, and
//! CDX-style `YYYYMMDD[HHMMSS]`) and relative forms (`now`, `today`,
//! `yesterday`, `3d`, `2w`, `6mo`, `1y`, `12h`, and `N days ago` / `N weeks
//! ago` / … variants). Relative values resolve against a `now` reference.

use chrono::{DateTime, Datelike, Duration, Months, NaiveDate, NaiveDateTime, TimeZone, Utc};

use crate::error::{Error, Result};

/// Parse a date/time expression against the current time.
pub fn parse(s: &str) -> Result<DateTime<Utc>> {
    parse_at(s, Utc::now())
}

/// Parse a date/time expression against an explicit `now` (for testing and
/// deterministic relative arithmetic).
pub fn parse_at(s: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
    let raw = s.trim();
    let lower = raw.to_ascii_lowercase();

    match lower.as_str() {
        "now" => return Ok(now),
        "today" => return Ok(start_of_day(now)),
        "yesterday" => return Ok(start_of_day(now) - Duration::days(1)),
        _ => {}
    }

    if let Some(dt) = parse_relative(&lower, now)? {
        return Ok(dt);
    }
    if let Some(dt) = parse_absolute(raw) {
        return Ok(dt);
    }
    Err(Error::Config(format!("unrecognized date/time: {raw}")))
}

fn start_of_day(dt: DateTime<Utc>) -> DateTime<Utc> {
    Utc.from_utc_datetime(&dt.date_naive().and_hms_opt(0, 0, 0).unwrap())
}

/// Parse relative forms; `Ok(None)` means "not a relative expression".
fn parse_relative(lower: &str, now: DateTime<Utc>) -> Result<Option<DateTime<Utc>>> {
    // Strip an optional trailing "ago" and a leading "-".
    let body = lower.strip_suffix("ago").map_or(lower, str::trim).trim();
    let body = body.strip_prefix('-').unwrap_or(body).trim();

    // Split into a leading number and a trailing unit, tolerating a space:
    // "3d", "3 d", "3 days", "2weeks".
    let digits: String = body.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return Ok(None);
    }
    let n: i64 = digits
        .parse()
        .map_err(|_| Error::Config(format!("invalid relative amount: {body}")))?;
    let unit = body[digits.len()..].trim();

    let dt = match unit {
        "h" | "hr" | "hrs" | "hour" | "hours" => now - Duration::hours(n),
        "d" | "day" | "days" => now - Duration::days(n),
        "w" | "wk" | "wks" | "week" | "weeks" => now - Duration::weeks(n),
        "mo" | "mon" | "mos" | "month" | "months" => {
            now - Months::new(u32::try_from(n).unwrap_or(0))
        }
        "y" | "yr" | "yrs" | "year" | "years" => {
            now - Months::new(u32::try_from(n).unwrap_or(0) * 12)
        }
        // Anything else (a bare number, or an absolute date like `2021-06`
        // whose "unit" is `-06`) is not a relative expression; let the absolute
        // parser try instead.
        _ => return Ok(None),
    };
    Ok(Some(dt))
}

/// Parse absolute forms.
fn parse_absolute(raw: &str) -> Option<DateTime<Utc>> {
    // RFC3339 / ISO-8601 with time zone.
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Some(dt.with_timezone(&Utc));
    }
    // `YYYY-MM-DDTHH:MM:SS` (no zone → assume UTC).
    if let Ok(ndt) = NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S") {
        return Some(Utc.from_utc_datetime(&ndt));
    }
    // CDX-style compact timestamps.
    for fmt in ["%Y%m%d%H%M%S", "%Y%m%d"] {
        if raw.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(ndt) = NaiveDateTime::parse_from_str(raw, fmt) {
                return Some(Utc.from_utc_datetime(&ndt));
            }
            if let Ok(nd) = NaiveDate::parse_from_str(raw, fmt) {
                return Some(Utc.from_utc_datetime(&nd.and_hms_opt(0, 0, 0).unwrap()));
            }
        }
    }
    // `YYYY-MM-DD`.
    if let Ok(nd) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        return Some(Utc.from_utc_datetime(&nd.and_hms_opt(0, 0, 0).unwrap()));
    }
    // `YYYY-MM` → first of month.
    if let Ok(nd) = NaiveDate::parse_from_str(&format!("{raw}-01"), "%Y-%m-%d") {
        return Some(Utc.from_utc_datetime(&nd.and_hms_opt(0, 0, 0).unwrap()));
    }
    // Bare `YYYY` → Jan 1.
    if raw.len() == 4 && raw.chars().all(|c| c.is_ascii_digit()) {
        if let Some(nd) = raw
            .parse::<i32>()
            .ok()
            .and_then(|y| NaiveDate::from_ymd_opt(y, 1, 1))
        {
            return Some(Utc.from_utc_datetime(&nd.and_hms_opt(0, 0, 0).unwrap()));
        }
    }
    None
}

/// The year of a parsed date (convenience for coarse crawl filtering).
#[must_use]
pub fn year_of(dt: DateTime<Utc>) -> u16 {
    u16::try_from(dt.year()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 11, 12, 0, 0).unwrap()
    }

    #[test]
    fn absolute_forms() {
        assert_eq!(parse_at("2021", now()).unwrap().year(), 2021);
        assert_eq!(parse_at("2021-06", now()).unwrap().month(), 6);
        assert_eq!(parse_at("2021-06-15", now()).unwrap().day(), 15);
        assert_eq!(parse_at("20210615", now()).unwrap().day(), 15);
        assert_eq!(parse_at("2021-06-15T08:30:00Z", now()).unwrap().hour(), 8);
    }

    #[test]
    fn relative_forms() {
        assert_eq!(parse_at("today", now()).unwrap(), start_of_day(now()));
        assert_eq!(
            parse_at("yesterday", now()).unwrap(),
            start_of_day(now()) - Duration::days(1)
        );
        assert_eq!(parse_at("3d", now()).unwrap(), now() - Duration::days(3));
        assert_eq!(
            parse_at("3 days ago", now()).unwrap(),
            now() - Duration::days(3)
        );
        assert_eq!(parse_at("2w", now()).unwrap(), now() - Duration::weeks(2));
        assert_eq!(parse_at("12h", now()).unwrap(), now() - Duration::hours(12));
        assert_eq!(parse_at("1y", now()).unwrap().year(), 2025);
        assert_eq!(parse_at("6mo", now()).unwrap().month(), 1);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_at("last tuesday", now()).is_err());
        assert!(parse_at("3 fortnights", now()).is_err());
    }
}
