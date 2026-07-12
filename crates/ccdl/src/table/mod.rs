//! Pure-Rust columnar backend over the cc-index Parquet table (feature `table`).
//!
//! Reads Common Crawl's cc-index Parquet directly from
//! `data.commoncrawl.org` over HTTPS (no AWS credentials), using row-group and
//! page pruning via column statistics and projecting only needed columns.

pub mod compile;
mod read;
pub mod store;

pub use compile::{TablePredicates, compile_predicates};
pub use read::read_parquet_bytes;

use std::sync::Arc;

use futures::StreamExt;
use object_store::{ObjectStore, path::Path as ObjPath};
use parquet::arrow::ParquetRecordBatchStreamBuilder;
use parquet::arrow::arrow_reader::ArrowReaderMetadata;
use parquet::arrow::async_reader::ParquetObjectReader;

use crate::error::{Error, Result};
use crate::index::CaptureStream;
use crate::model::crawl::CrawlId;
use crate::model::query::UrlQuery;
use crate::registry::CrawlInfo;

/// Reads the cc-index columnar table.
pub struct TableClient {
    store: Arc<dyn ObjectStore>,
}

impl TableClient {
    /// Build a client over the free HTTPS `data.commoncrawl.org` endpoint.
    pub fn http() -> Result<Self> {
        let store = object_store::http::HttpBuilder::new()
            .with_url("https://data.commoncrawl.org")
            .build()
            .map_err(|e| Error::Backend(format!("object_store: {e}")))?;
        Ok(TableClient {
            store: Arc::new(store),
        })
    }

    fn partition_path(crawl: &CrawlId) -> ObjPath {
        ObjPath::from(format!(
            "cc-index/table/cc-main/warc/crawl={}/subset=warc",
            crawl.0
        ))
    }

    /// List the Parquet objects in a crawl's warc partition.
    pub async fn list_parquet(&self, crawl: &CrawlId) -> Result<Vec<ObjPath>> {
        let prefix = Self::partition_path(crawl);
        let mut stream = self.store.list(Some(&prefix));
        let mut paths = Vec::new();
        while let Some(meta) = stream.next().await {
            let meta = meta.map_err(|e| Error::Backend(format!("list: {e}")))?;
            if meta.location.as_ref().ends_with(".parquet") {
                paths.push(meta.location);
            }
        }
        Ok(paths)
    }

    /// Search one crawl's Parquet partition, streaming matching captures with
    /// row-group pruning via `url_surtkey` statistics.
    pub async fn search_crawl(
        &self,
        crawl: &CrawlInfo,
        query: &UrlQuery,
    ) -> Result<Vec<crate::model::capture::Capture>> {
        let preds = compile_predicates(query);
        let paths = self.list_parquet(&crawl.id).await?;
        let mut out = Vec::new();
        for path in paths {
            let meta = self
                .store
                .head(&path)
                .await
                .map_err(|e| Error::Backend(format!("head: {e}")))?;
            let reader = ParquetObjectReader::new(self.store.clone(), meta);
            let arrow_meta = ArrowReaderMetadata::load_async(
                &mut reader.clone(),
                parquet::arrow::arrow_reader::ArrowReaderOptions::default(),
            )
            .await
            .map_err(|e| Error::Backend(format!("parquet meta: {e}")))?;
            let builder = ParquetRecordBatchStreamBuilder::new_with_metadata(reader, arrow_meta);
            let mut stream = builder
                .build()
                .map_err(|e| Error::Backend(format!("parquet stream: {e}")))?;
            while let Some(batch) = stream.next().await {
                let batch = batch.map_err(|e| Error::Backend(format!("batch: {e}")))?;
                read::collect_batch_pub(&crawl.id, &batch, &preds, &mut out);
            }
        }
        Ok(out)
    }

    /// Search across crawls, returning a buffered capture stream.
    pub async fn search(&self, crawls: Vec<CrawlInfo>, query: UrlQuery) -> Result<CaptureStream> {
        let mut all = Vec::new();
        for crawl in crawls {
            all.extend(self.search_crawl(&crawl, &query).await?);
        }
        Ok(Box::pin(futures::stream::iter(all.into_iter().map(Ok))))
    }
}
