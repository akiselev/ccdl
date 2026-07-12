//! Output writers: ndjson (default, agent contract), csv, table.

use std::io::{self, Write};

use ccdl::model::capture::Capture;

/// Supported output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Newline-delimited JSON (default).
    Ndjson,
    /// CSV with a stable header.
    Csv,
    /// Human-friendly aligned table.
    Table,
}

impl Format {
    /// Parse a format token; unknown tokens fall back to ndjson.
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "csv" => Format::Csv,
            "table" => Format::Table,
            _ => Format::Ndjson,
        }
    }
}

/// A streaming capture writer.
pub struct Writer {
    format: Format,
    wrote_header: bool,
}

impl Writer {
    /// Create a writer for a format.
    #[must_use]
    pub fn new(format: Format) -> Self {
        Writer {
            format,
            wrote_header: false,
        }
    }

    /// Write one capture.
    pub fn write(&mut self, out: &mut impl Write, c: &Capture) -> io::Result<()> {
        match self.format {
            Format::Ndjson => {
                let line = serde_json::to_string(c).map_err(io::Error::other)?;
                writeln!(out, "{line}")
            }
            Format::Csv => {
                if !self.wrote_header {
                    writeln!(
                        out,
                        "crawl,timestamp,status,url,digest,filename,offset,length"
                    )?;
                    self.wrote_header = true;
                }
                writeln!(
                    out,
                    "{},{},{},{},{},{},{},{}",
                    c.crawl,
                    c.timestamp.format("%Y%m%d%H%M%S"),
                    c.status,
                    csv_escape(&c.url),
                    c.digest,
                    csv_escape(&c.filename),
                    c.offset,
                    c.length
                )
            }
            Format::Table => {
                writeln!(
                    out,
                    "{:<16} {:<14} {:>3}  {}",
                    c.crawl,
                    c.timestamp.format("%Y%m%d%H%M%S"),
                    c.status,
                    c.url
                )
            }
        }
    }
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}
