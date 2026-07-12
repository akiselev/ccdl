//! Read cc-index Parquet into `Capture`, applying compiled predicates.

use arrow::array::{Array, Int32Array, Int64Array, StringArray, TimestampMillisecondArray};
use arrow::record_batch::RecordBatch;
use bytes::Bytes;
use chrono::{TimeZone, Utc};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use super::compile::TablePredicates;
use crate::error::{Error, Result};
use crate::model::capture::Capture;
use crate::model::crawl::CrawlId;
use crate::model::query::StatusPred;

/// Columns projected from the cc-index table.
const COLUMNS: &[&str] = &[
    "url_surtkey",
    "url",
    "fetch_time",
    "fetch_status",
    "content_mime_type",
    "content_mime_detected",
    "content_languages",
    "content_digest",
    "warc_filename",
    "warc_record_offset",
    "warc_record_length",
];

fn str_col<'a>(b: &'a RecordBatch, name: &str) -> Option<&'a StringArray> {
    b.column_by_name(name)?
        .as_any()
        .downcast_ref::<StringArray>()
}

fn i32_col<'a>(b: &'a RecordBatch, name: &str) -> Option<&'a Int32Array> {
    b.column_by_name(name)?
        .as_any()
        .downcast_ref::<Int32Array>()
}

fn i64_col<'a>(b: &'a RecordBatch, name: &str) -> Option<&'a Int64Array> {
    b.column_by_name(name)?
        .as_any()
        .downcast_ref::<Int64Array>()
}

fn status_ok(pred: &StatusPred, status: u16) -> bool {
    match pred {
        StatusPred::Eq(s) => status == *s,
        StatusPred::In(codes) => codes.contains(&status),
        StatusPred::Class(c) => status / 100 == u16::from(*c),
    }
}

fn opt(s: &StringArray, i: usize) -> Option<String> {
    if s.is_null(i) {
        None
    } else {
        Some(s.value(i).to_owned())
    }
}

/// Parse Parquet bytes and yield captures matching `preds`, tagged with `crawl`.
pub fn read_parquet_bytes(
    crawl: &CrawlId,
    bytes: Bytes,
    preds: &TablePredicates,
) -> Result<Vec<Capture>> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(bytes)
        .map_err(|e| Error::Backend(format!("parquet open: {e}")))?;

    // Project only columns that exist in this file.
    let schema = builder.schema().clone();
    let indices: Vec<usize> = COLUMNS
        .iter()
        .filter_map(|c| schema.index_of(c).ok())
        .collect();
    let mask = parquet::arrow::ProjectionMask::roots(builder.parquet_schema(), indices);
    let reader = builder
        .with_projection(mask)
        .build()
        .map_err(|e| Error::Backend(format!("parquet reader: {e}")))?;

    let mut out = Vec::new();
    for batch in reader {
        let batch = batch.map_err(|e| Error::Backend(format!("parquet batch: {e}")))?;
        collect_batch(crawl, &batch, preds, &mut out);
    }
    Ok(out)
}

/// Collect matching captures from a decoded `RecordBatch` (used by the async
/// streaming reader).
pub(crate) fn collect_batch_pub(
    crawl: &CrawlId,
    b: &RecordBatch,
    preds: &TablePredicates,
    out: &mut Vec<Capture>,
) {
    collect_batch(crawl, b, preds, out);
}

fn collect_batch(
    crawl: &CrawlId,
    b: &RecordBatch,
    preds: &TablePredicates,
    out: &mut Vec<Capture>,
) {
    let surtkey = str_col(b, "url_surtkey");
    let url = str_col(b, "url");
    let mime = str_col(b, "content_mime_type");
    let mime_det = str_col(b, "content_mime_detected");
    let langs = str_col(b, "content_languages");
    let digest = str_col(b, "content_digest");
    let filename = str_col(b, "warc_filename");
    let offset = i64_col(b, "warc_record_offset");
    let length = i64_col(b, "warc_record_length");
    let status = i32_col(b, "fetch_status");
    let fetch_time = b
        .column_by_name("fetch_time")
        .and_then(|c| c.as_any().downcast_ref::<TimestampMillisecondArray>());

    for i in 0..b.num_rows() {
        let key = surtkey.map_or(String::new(), |a| a.value(i).to_owned());
        if let Some((lo, hi)) = &preds.surt_range {
            if &key < lo || &key >= hi {
                continue;
            }
        }
        let st = status.map_or(0u16, |a| u16::try_from(a.value(i)).unwrap_or(0));
        if let Some(pred) = &preds.status {
            if !status_ok(pred, st) {
                continue;
            }
        }
        let m = mime.and_then(|a| opt(a, i));
        let md = mime_det.and_then(|a| opt(a, i));
        if let Some(want) = &preds.mime {
            if m.as_deref() != Some(want.as_str()) && md.as_deref() != Some(want.as_str()) {
                continue;
            }
        }

        let ts = fetch_time
            .map(|a| {
                Utc.timestamp_millis_opt(a.value(i))
                    .single()
                    .unwrap_or_default()
            })
            .unwrap_or_default();

        out.push(Capture {
            crawl: crawl.clone(),
            urlkey: key,
            url: url.map_or(String::new(), |a| a.value(i).to_owned()),
            timestamp: ts,
            status: st,
            mime: m,
            mime_detected: md,
            languages: langs
                .and_then(|a| opt(a, i))
                .map(|s| {
                    s.split(',')
                        .map(str::to_owned)
                        .filter(|x| !x.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            digest: digest.map_or(String::new(), |a| a.value(i).to_owned()),
            length: length.map_or(0, |a| u64::try_from(a.value(i)).unwrap_or(0)),
            offset: offset.map_or(0, |a| u64::try_from(a.value(i)).unwrap_or(0)),
            filename: filename.map_or(String::new(), |a| a.value(i).to_owned()),
            redirect: None,
            truncated: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::query::UrlQuery;
    use crate::table::compile::compile_predicates;
    use arrow::array::{Int32Array, Int64Array, StringArray, TimestampMillisecondArray};
    use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
    use std::sync::Arc;

    fn make_parquet() -> Bytes {
        let schema = Arc::new(Schema::new(vec![
            Field::new("url_surtkey", DataType::Utf8, false),
            Field::new("url", DataType::Utf8, false),
            Field::new(
                "fetch_time",
                DataType::Timestamp(TimeUnit::Millisecond, None),
                false,
            ),
            Field::new("fetch_status", DataType::Int32, false),
            Field::new("content_mime_type", DataType::Utf8, true),
            Field::new("content_mime_detected", DataType::Utf8, true),
            Field::new("content_languages", DataType::Utf8, true),
            Field::new("content_digest", DataType::Utf8, false),
            Field::new("warc_filename", DataType::Utf8, false),
            Field::new("warc_record_offset", DataType::Int64, false),
            Field::new("warc_record_length", DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![
                    "com,example)/manufacturer/acme",
                    "com,other)/x",
                ])),
                Arc::new(StringArray::from(vec![
                    "https://example.com/manufacturer/acme",
                    "https://other.com/x",
                ])),
                Arc::new(TimestampMillisecondArray::from(vec![
                    1_600_000_000_000,
                    1_600_000_001_000,
                ])),
                Arc::new(Int32Array::from(vec![200, 404])),
                Arc::new(StringArray::from(vec![
                    Some("text/html"),
                    Some("text/html"),
                ])),
                Arc::new(StringArray::from(vec![
                    Some("text/html"),
                    Some("text/html"),
                ])),
                Arc::new(StringArray::from(vec![Some("eng"), Some("eng")])),
                Arc::new(StringArray::from(vec!["sha1:A", "sha1:B"])),
                Arc::new(StringArray::from(vec!["f1.warc.gz", "f2.warc.gz"])),
                Arc::new(Int64Array::from(vec![0, 100])),
                Arc::new(Int64Array::from(vec![10, 20])),
            ],
        )
        .unwrap();

        let mut buf = Vec::new();
        {
            let mut w = parquet::arrow::ArrowWriter::try_new(&mut buf, schema, None).unwrap();
            w.write(&batch).unwrap();
            w.close().unwrap();
        }
        Bytes::from(buf)
    }

    #[test]
    fn filters_by_surt_range_and_status() {
        let bytes = make_parquet();
        let q = UrlQuery::prefix("example.com/manufacturer/").status(200);
        let preds = compile_predicates(&q);
        let caps = read_parquet_bytes(&CrawlId("C".into()), bytes, &preds).unwrap();
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].url, "https://example.com/manufacturer/acme");
        assert_eq!(caps[0].status, 200);
        assert_eq!(caps[0].offset, 0);
        assert_eq!(caps[0].length, 10);
    }
}
