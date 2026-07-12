# ccdl — Common Crawl Downloader

`ccdl` is a batteries-included Rust library and CLI for **discovering, searching,
enumerating, and fetching** Common Crawl (CC) captures. It is transport- and
storage-aware, polite by default, aggressively cached, and designed to be driven
both by application code and by autonomous scraping agents.

The motivating use case is [foundry](../foundry) / PartFoundry: reconstructing
historical and recent manufacturer and distributor catalogs from CC without
re-crawling the live web and without fighting anti-bot systems (Cloudflare, rate
limits, CAPTCHAs). A representative task:

> Given `example.com/manufacturer/<name>/<part>`, enumerate every captured URL
> matching that shape across crawls, dedupe by content, and fetch the bytes of a
> chosen sample.

---

## 1. What Common Crawl actually offers (and doesn't)

Understanding the substrate drives the whole design.

### 1.1 The three access surfaces

| Surface | Location | Good for | Cost / auth |
|---|---|---|---|
| **CDX index (HTTP)** | `https://index.commoncrawl.org/CC-MAIN-YYYY-WW-index?url=…` | Targeted lookups & prefix/host/domain enumeration within a crawl; interactive latency | Free, no auth; **rate-limited**, "please don't overload" |
| **Columnar / URL index (Parquet)** | `s3://commoncrawl/cc-index/table/cc-main/warc/` (also over `https://data.commoncrawl.org/…`) | **Bulk** enumeration/filtering/aggregation; cross-crawl; SQL predicates | Free over HTTPS mirror; S3 is **requester-pays**; ~300 GB/crawl |
| **WARC/WAT/WET payloads** | `https://data.commoncrawl.org/<warc_filename>` (byte-range) | Fetching the actual captured bytes | Free HTTPS (CloudFront); S3 requester-pays |

The **collection registry** lives at
`https://index.commoncrawl.org/collinfo.json` (~100 monthly crawls since 2013,
~one every few weeks; each has an `id`, `name`, `cdx-api` URL, and time bounds).

### 1.2 What "search" can mean here

Full-text search is **not** available. What *is* available, and what `ccdl`
exposes as first-class "search":

- **URL / host / domain / path-prefix enumeration** (`matchType` =
  `exact|prefix|host|domain`) — the core enumeration primitive.
- **Field filters**: HTTP status, MIME (declared *and* detected), content
  language, TLD, URL regex.
- **SURT-key range scans** (columnar) — arbitrary lexicographic URL ranges.
- **Digest collapse / dedup** — one row per unique content hash.
- **Temporal enumeration** — the same URL across crawls, with change detection
  by `content_digest`.
- **Seed discovery** — pull captured `robots.txt` / `sitemap.xml` to bootstrap
  enumeration of a site whose URL scheme is unknown.
- **Template extraction** — given `host/manufacturer/{name}/{part}`, bind the
  variables out of enumerated URLs.

### 1.3 Limits to encode in the API (not hide)

- Coverage is **partial and non-uniform**; first/last capture are *observation
  boundaries*, not lifecycle events. `ccdl` never invents dates.
- CC may have captured a **Cloudflare/anti-bot challenge page** instead of real
  content. Fetches therefore always surface `fetch_status` and a
  `looks_like_challenge()` heuristic; `status:200` filtering is the default.
- JS-rendered data absent from the captured HTML is simply gone.
- CDX prefix/domain queries can be **huge**; always check size first
  (`showNumPages`) and prefer the columnar backend for bulk.

---

## 2. Design goals

1. **Enumeration-first.** The headline verb is "list every capture matching a
   URL shape," not "download a crawl."
2. **Batteries included & ergonomic.** A three-line happy path; sensible
   defaults; builder configuration; async streams that don't blow up memory on
   million-row prefixes.
3. **Polite by construction.** Global rate limiting, bounded concurrency,
   backoff on 503, a real `User-Agent`. It should be *hard* to hammer CC.
4. **Cache everything expensive.** Registry, index responses, and payloads —
   content-addressed, configurable, inspectable.
5. **Agent-friendly.** Deterministic, structured (NDJSON) output; a stable CLI
   contract; resumable manifests; explicit budgets.
6. **Backend-pluggable.** CDX for targeted work, columnar for bulk, without the
   caller rewriting queries.
7. **Honest.** Distinguish "not captured" from "captured but non-200"; carry
   provenance (crawl id, digest, WARC locator) on every record.

Non-goals: live crawling, JS rendering, a WARC *writer*, or being the product's
authority layer (that stays in foundry).

---

## 3. Architecture

```
                        ┌────────────────────────────────────────┐
                        │                ccdl-cli                 │
                        │  crawls · search · enumerate · manifest │
                        │  · fetch · text · sitemap · cache       │
                        └───────────────────┬────────────────────┘
                                            │
                        ┌───────────────────▼────────────────────┐
   High-level facade →  │                  Ccdl                   │
                        │  search() enumerate() manifest()        │
                        │  fetch() text() registry() cache()      │
                        └───┬───────────┬───────────┬─────────────┘
                            │           │           │
          ┌─────────────────▼──┐ ┌──────▼───────┐ ┌─▼──────────────┐
Backends →│  IndexClient (CDX) │ │ TableClient  │ │  WarcClient    │
          │  HTTP JSON/CDXJ    │ │ (columnar)   │ │  byte-range    │
          └─────────┬──────────┘ └──────┬───────┘ └───────┬────────┘
                    │                   │                 │
          ┌─────────▼───────────────────▼─────────────────▼────────┐
Cross-   │  Registry · Cache · Politeness (rate limit/retry) ·      │
cutting →│  HTTP transport · SURT canonicalization · error model    │
          └──────────────────────────────────────────────────────────┘

Core (I/O-free): CrawlId · UrlQuery · MatchType · Filter · Capture ·
                 Manifest · Sampling · UrlTemplate
```

### 3.1 Crates / feature layout

Single workspace, two published crates, feature-gated heavy backends:

- **`ccdl`** — library.
  - default features: `index` (CDX), `warc` (fetch+parse), `cache-fs`, `rustls`.
  - `table` — pure-Rust columnar backend (`arrow`/`parquet` + `object_store`;
    see §7). Off by default; no C dependencies.
  - `athena` — optional, additive AWS Athena adapter for very large jobs.
- **`ccdl-cli`** — thin binary (`ccdl`) over the library; `clap`-based.

Core domain types live in a `ccdl::model` module with **no I/O deps**, so they
can be shared/serialized by callers (foundry stores them). This generalizes the
existing `partfoundry-commoncrawl` primitives (`CaptureLocator`, `IndexQuery`,
`MatchType`, sampling policy, domain epochs) — see §9.

---

## 4. Core data model (`ccdl::model`)

```rust
/// A monthly crawl, e.g. "CC-MAIN-2026-30".
pub struct CrawlId(pub String);

/// How to select which crawls a query spans.
pub enum CrawlSelector {
    All,
    Latest,
    LatestN(usize),
    Ids(Vec<CrawlId>),
    Since(chrono::NaiveDate),
    YearRange { from: u16, to: u16 },
}

pub enum MatchType { Exact, Prefix, Host, Domain }

/// One capture row from an index (CDX row or columnar row), backend-agnostic.
pub struct Capture {
    pub crawl: CrawlId,
    pub urlkey: String,          // SURT, e.g. "com,example)/manufacturer/acme/x1"
    pub url: String,
    pub timestamp: DateTime<Utc>,
    pub status: u16,             // fetch_status
    pub mime: Option<String>,           // declared
    pub mime_detected: Option<String>,  // sniffed
    pub languages: Vec<String>,
    pub digest: String,          // "sha1:…" content hash
    pub length: u64,             // warc_record_length
    pub offset: u64,             // warc_record_offset
    pub filename: String,        // warc_filename (relative)
    pub redirect: Option<String>,
    pub truncated: Option<String>,
}

impl Capture {
    pub fn warc_url(&self) -> String;      // https://data.commoncrawl.org/<filename>
    pub fn range_header(&self) -> String;  // bytes=offset-(offset+length-1)
    pub fn capture_key(&self) -> String;   // stable dedup id
    pub fn looks_like_challenge(&self) -> bool; // status/mime heuristic
}

/// A resolved, possibly-sampled set of captures for a query.
pub struct Manifest { pub query: UrlQuery, pub captures: Vec<Capture> }

pub enum Sampling {
    All,
    UniqueDigest,                 // one per distinct content hash
    FirstLast,
    FirstLastAndDigestChanges,    // foundry default
    AdaptiveChangePoint { max_per_url: u32 },
}
```

### 4.1 Query model

`UrlQuery` is the single description that both backends compile:

```rust
pub struct UrlQuery {
    pub pattern: String,               // "example.com/manufacturer/"
    pub match_type: MatchType,
    pub crawls: CrawlSelector,
    pub filters: Vec<Filter>,
    pub collapse: Option<Collapse>,    // Digest | UrlKey | Timestamp(n)
    pub fields: Option<Vec<Field>>,    // projection
    pub time_range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    pub limit: Option<usize>,
}

pub enum Filter {
    Status(StatusPred),        // == 200, in {200,301}, class 2xx
    Mime(String),              // declared or detected
    Language(String),
    Tld(String),
    UrlRegex(String),
}
```

- The **CDX compiler** maps these to `url=`, `matchType=`, repeated `filter=`
  (with `!`/regex forms), `collapse=`, `fl=`, `from=`/`to=`, `limit=`, and
  drives `page`/`pageSize`/`showNumPages` pagination.
- The **columnar compiler** maps the same query to a SQL `WHERE` over the
  partitioned Parquet (`crawl`, `subset='warc'`, `url_host_name`, `url_path LIKE
  …`, `fetch_status`, `content_mime_detected`, …), with SURT-key range pushdown
  for prefixes.

Callers rarely build `UrlQuery` by hand — see the builders in §5.

---

## 5. High-level API (the ergonomic path)

```rust
let cc = Ccdl::builder()
    .cache_dir("~/.cache/ccdl")     // default XDG cache
    .max_rps(3.0)                   // polite default
    .max_concurrency(8)
    .user_agent("foundry-ccarch/0.1 (+https://…)")
    .build()
    .await?;

// (a) Enumerate distinct URLs under a path prefix, across the last 6 crawls.
let urls: Vec<String> = cc
    .enumerate("example.com/manufacturer/", MatchType::Prefix)
    .crawls(CrawlSelector::LatestN(6))
    .status(200)
    .distinct_urls()
    .await?;

// (b) Stream captures for a domain without buffering millions of rows.
let mut stream = cc
    .search(UrlQuery::domain("example.com").status_class(2))
    .stream()
    .await?;
while let Some(cap) = stream.try_next().await? {
    println!("{} {} {}", cap.timestamp, cap.status, cap.url);
}

// (c) Build a sampled manifest (first/last + every content change).
let manifest = cc
    .manifest(UrlQuery::prefix("example.com/manufacturer/"))
    .crawls(CrawlSelector::All)
    .sample(Sampling::FirstLastAndDigestChanges)
    .await?;

// (d) Fetch the actual bytes of a chosen capture (byte-range → gunzip → parse).
let record = cc.fetch(&manifest.captures[0]).await?;
let http   = record.http_response()?;   // status line, headers, body
let html   = http.text()?;

// (e) Template extraction for the manufacturer/<name>/<part> case.
let tmpl = UrlTemplate::parse("example.com/manufacturer/{name}/{part}")?;
let rows = cc.enumerate_template(&tmpl).crawls(All).bind().await?;
// -> Vec<{ name: "acme", part: "x1", captures: [...] }>
```

`Ccdl` auto-selects the backend by query shape and configuration: exact/host and
small prefix queries → CDX; large prefix/domain or explicitly `.bulk()` →
columnar when the `table` feature is enabled, else paginated CDX with a warning.

---

## 6. Cross-cutting subsystems

### 6.1 Caching (`ccdl::cache`)

A `Cache` trait with layered, content-addressed implementations:

```rust
#[async_trait]
pub trait Cache: Send + Sync {
    async fn get(&self, key: &CacheKey) -> Result<Option<CacheHit>>;
    async fn put(&self, key: &CacheKey, bytes: Bytes, meta: EntryMeta) -> Result<()>;
}
```

Three cache *domains*, each independently configurable (TTL, size cap,
enable/disable):

1. **Registry** — `collinfo.json`, long TTL, refreshable.
2. **Index responses** — keyed by the *normalized* query + crawl + page. Lets
   repeated/resumed enumerations skip the network.
3. **Payloads** — **content-addressed by `content_digest`**, so identical
   captures across crawls are stored once (this is the big win; CC is full of
   unchanged pages). Stored as the raw gzip'd WARC record slice.

Backends: `FsCache` (default, CAS directory layout `ab/cd/<digest>`),
`MemoryCache` (moka), `NoopCache`, and `LayeredCache(mem, fs)`. All pluggable so
foundry can back it with its own content-addressed artifact store.

### 6.2 Politeness (`ccdl::polite`)

- Token-bucket rate limiter (`governor`), default ~2–3 req/s, separate buckets
  for `index.commoncrawl.org` vs `data.commoncrawl.org`.
- Bounded global concurrency (semaphore).
- Retry with exponential backoff + jitter, honoring `Retry-After`; treats 503 as
  the expected overload signal, not a hard error.
- Mandatory descriptive `User-Agent`; refuses to run with a placeholder.

### 6.3 WARC fetch/parse (`ccdl::warc`)

Each record in a CC WARC is an **independent gzip member**, so a byte-range slice
is itself a valid gzip stream. Pipeline: range GET → gunzip (`flate2`) → parse
WARC headers → parse the enclosed HTTP response (`httparse`) → expose
status/headers/decoded body. Reuses the `warc` crate for record parsing where it
fits; a thin `HttpCapture` wraps the payload with charset/`content-encoding`
handling. WAT (metadata) and WET (extracted text) fetching share the locator
path.

### 6.4 SURT & canonicalization (`ccdl::surt`)

Canonical SURT keys are needed to compute `urlkey`, to dedup across backends, and
to do lexicographic range scans on the columnar `url_surtkey`. Implemented to
match CC/pywb canonicalization (host reversal, default-port stripping, query sort
options).

### 6.5 Errors (`ccdl::error`)

`thiserror`-based `Error` distinguishing: `RateLimited`, `NotFound`,
`Http(status)`, `IndexTooLarge { pages }`, `Parse`, `Cache`, `Config`,
`BackendUnavailable`. Enumeration returning zero rows is `Ok(empty)`, never an
error — "not captured" is a valid answer.

---

## 7. The columnar backend — pure Rust (decided)

Bulk enumeration (`example.com/*`, `url_path LIKE '/manufacturer/%'` across all
crawls) is painful over CDX and cheap over the Parquet index. The `table` feature
implements this in **pure Rust** — no C/C++ dependency, a lean binary:

- **Read path:** `object_store` (its HTTP backend) points at the free HTTPS
  mirror `https://data.commoncrawl.org/cc-index/table/cc-main/warc/
  crawl=<id>/subset=warc/…`, so **no AWS account or credentials** are required.
  S3 (`object_store` S3 backend, requester-pays) is an opt-in alternative for
  users inside AWS.
- **Query path:** the `arrow`/`parquet` reader with **row-group + page pruning**
  driven by Parquet column statistics. The columnar compiler (§4.1) turns a
  `UrlQuery` into predicates we push down: partition selection on `crawl` /
  `subset`, `url_host_name`/`url_host_tld` equality, `url_surtkey` range bounds
  for path prefixes, and `fetch_status`/`content_mime_detected` filters, with a
  residual row-level filter for anything statistics can't prune (e.g. regex).
- **Trade-off accepted:** we hand-roll predicate pushdown that an embedded SQL
  engine would give for free, in exchange for zero native deps and a small
  static binary. The `TableClient` is a trait, so an embedded-engine or Athena
  implementation can be dropped in later without touching callers.
- **Athena** remains an optional, additive `athena` feature for jobs too large to
  stream locally (submit SQL to the user's AWS account, ~$1.50/crawl scan bound).

Getting `url_surtkey` bounds right for a given path prefix depends on correct
SURT canonicalization (§6.4); the prefix `host/path/` becomes a
`[surt, surt+0xFF]` half-open range for pruning.

---

## 8. CLI (`ccdl-cli`)

NDJSON-first so it composes with `jq`, agents, and pipelines. Every subcommand
takes `--crawls`, `--cache-dir`, `--max-rps`, `-o {ndjson,csv,jsonl,table}`.

```
ccdl crawls [--refresh]                        # list crawls (registry)
ccdl stats  <pattern> [--match prefix]         # showNumPages / row estimate
ccdl search <pattern> [--match prefix|host|domain]
            [--status 200] [--mime text/html] [--collapse digest]
            [--crawls latest-6|all|2020..2024]
ccdl enumerate <pattern> --distinct            # just the distinct URLs
ccdl manifest  <pattern> --sample first-last-digest -o manifest.ndjson
ccdl fetch  <url|@manifest.ndjson> [--crawl …] [--out dir/] [--text]
ccdl text   <url>                              # WET / extracted text
ccdl sitemap <host>                            # seed discovery via robots/sitemap
ccdl cache  (path | stats | clear | gc)
```

Examples:

```bash
# Every distinct product URL shape for a distributor, last 8 crawls:
ccdl enumerate 'octopart-like.example/product/' --match prefix \
     --crawls latest-8 --status 200 --distinct -o ndjson > urls.ndjson

# Build a deduped historical manifest, then fetch one sample per content change:
ccdl manifest 'example.com/manufacturer/' --crawls all \
     --sample first-last-digest -o manifest.ndjson
ccdl fetch @manifest.ndjson --out ./captures/ --text
```

The CLI is a stable contract for agents: deterministic ordering, explicit exit
codes, `--budget` caps (max requests / max bytes / max pages), and
`--dry-run` that prints the compiled backend query without executing.

---

## 9. Relationship to foundry / `partfoundry-commoncrawl` (decided)

Foundry adopts `ccdl` as a dependency and **retires the overlapping primitives**.
The dependency-free `partfoundry-commoncrawl` types migrate as follows:

| foundry type | disposition |
|---|---|
| `CaptureLocator` | replaced by `ccdl::model::Capture` (superset) |
| `IndexQuery` / `MatchType` | replaced by `ccdl::model::{UrlQuery, MatchType}` |
| `HistoricalSamplingPolicy` | replaced by `ccdl::model::Sampling` |
| `ManufacturerDomainEpoch` | **stays in foundry** (product-domain semantics) |

`partfoundry-commoncrawl` becomes a thin adapter crate: it re-exports the needed
`ccdl` types and keeps only foundry-specific concepts (manufacturer/domain
epochs, product-authority rules, budget policy) that map onto `ccdl`'s generic
mechanisms (`Sampling`, `CrawlSelector`, temporal enumeration). One source of
truth for the CC mechanics; product meaning stays in foundry. Note foundry's
workspace is currently dependency-free by policy — adopting `ccdl` is its first
third-party dependency and should be recorded as an ADR there.

---

## 10. Dependencies (proposed)

`tokio`, `reqwest` (rustls), `serde`/`serde_json`, `url`, `chrono`, `bytes`,
`flate2`, `httparse`, `futures`, `governor`, `thiserror`, `tracing`; optional
`moka` (mem cache), `arrow` + `parquet` + `object_store` (`table`, pure-Rust),
`aws-sdk-athena` (`athena`), `clap` + `indicatif` (CLI). Async-only core (no
`blocking` facade for now). `ccdl` forbids `unsafe`, matching foundry's lints.

---

## 11. Decisions

Resolved:

1. **Columnar backend** (§7): **pure-Rust** (`arrow`/`parquet` + `object_store`),
   feature-gated `table`, off by default; Athena additive.
2. **foundry relationship** (§9): **foundry depends on `ccdl`** and retires the
   overlapping primitives.
3. **Sync facade**: **async-only** to start; `blocking` deferred.

Still open:

4. **Cache default location/format**: XDG `~/.cache/ccdl` CAS as proposed, or
   back onto foundry's artifact store from the start? (Defaulting to the
   pluggable `FsCache`; foundry can inject its own `Cache` impl.)

## 12. Milestones

1. **M1 — Registry + CDX enumeration + FS cache.** `crawls`, `search`,
   `enumerate`, `stats`; streaming pagination; politeness. Covers the headline
   use case for a single/handful of crawls.
2. **M2 — Fetch + WARC/HTTP parse + payload CAS + `manifest`/`fetch`/`text`.**
3. **M3 — Pure-Rust columnar backend (`table`) + sampling policies + template
   extraction.** Bulk cross-crawl enumeration with SURT-range/statistics
   pruning.
4. **M4 — Sitemap seed discovery, budgets, Athena adapter, foundry integration
   (adopt `ccdl`, retire overlap, add ADR).**
