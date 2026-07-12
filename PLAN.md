# ccdl — Implementation Plan

Actionable build plan for the architecture in [DESIGN.md](./DESIGN.md). Ordered by
milestone; each phase lists concrete files, the public surface it introduces,
tests, and acceptance criteria. Section references (§) point at DESIGN.md.

Guiding rules:

- Every phase ends **green**: `cargo test --workspace`, `cargo clippy
  --all-targets --all-features -D warnings`, `cargo fmt --check`.
- Network-touching tests are gated behind `#[ignore]` + a `CCDL_LIVE=1` env
  guard; the default test run is hermetic and uses recorded fixtures.
- Public API is additive after M1; breaking changes only between milestones.
- `unsafe` is forbidden (matches foundry lints).

---

## Phase 0 — Workspace bootstrap

**Goal:** compiling empty workspace with lints, CI, and the dependency skeleton.

Files:

```
Cargo.toml                     # workspace, resolver=2, shared lints, dep versions
rust-toolchain.toml            # pinned stable (edition 2024, rust-version 1.85+)
.github/workflows/ci.yml       # fmt + clippy + test (hermetic) matrix
crates/ccdl/Cargo.toml         # lib, features declared, deps behind features
crates/ccdl/src/lib.rs         # module tree stubs + crate docs
crates/ccdl-cli/Cargo.toml     # bin "ccdl", depends on ccdl
crates/ccdl-cli/src/main.rs    # clap skeleton, `ccdl --version`
```

Tasks:

- [ ] Workspace `Cargo.toml`: members, `[workspace.package]` (version 0.0.1,
      edition 2024), `[workspace.lints]` (`unsafe_code = "forbid"`, clippy
      `all`/`pedantic` = warn), `[workspace.dependencies]` pinning every crate
      from DESIGN §10.
- [ ] Declare `ccdl` features: `default = ["index","warc","cache-fs","rustls"]`,
      plus `table`, `athena`, `moka`. Gate optional deps with `optional = true`
      and `dep:`.
- [ ] `lib.rs` module tree (all stubbed): `model`, `error`, `registry`,
      `polite`, `cache`, `surt`, `index`, `warc`, `table` (cfg), `athena` (cfg),
      `client` (the `Ccdl` facade), `prelude`.
- [ ] CLI: clap derive skeleton with subcommand enum (no logic yet), global
      flags (`--cache-dir`, `--max-rps`, `--crawls`, `-o/--output`,
      `--user-agent`, `-v`).

**Acceptance:** `cargo build --workspace`, `cargo run -p ccdl-cli -- --version`,
CI green.

---

## Phase 1 (M1) — Registry + CDX enumeration + FS cache + core CLI

Delivers the headline use case: enumerate `example.com/manufacturer/` across a
handful of crawls, deduped, as NDJSON.

### 1a. Error model — `src/error.rs`

- [ ] `Error` (thiserror) variants: `RateLimited{retry_after}`, `NotFound`,
      `Http{status}`, `IndexTooLarge{pages}`, `Parse(String)`, `Cache(String)`,
      `Config(String)`, `Backend(String)`, `Io`, `Network`. `Result<T>` alias.
- [ ] Empty enumeration is `Ok`, never an error (DESIGN §6.5).

### 1b. Core model — `src/model/` (I/O-free, `serde`)

- [ ] `crawl.rs`: `CrawlId`, `CrawlSelector` (`All|Latest|LatestN|Ids|Since|
      YearRange`) with `Display`/`FromStr` (parse `latest-6`, `2020..2024`,
      `all`, `CC-MAIN-…`).
- [ ] `query.rs`: `MatchType`, `Filter`, `StatusPred`, `Collapse`, `Field`,
      `UrlQuery` + ergonomic constructors (`UrlQuery::prefix/host/domain/exact`)
      and builder methods (`.status()`, `.status_class()`, `.mime()`,
      `.collapse()`, `.crawls()`, `.limit()`, `.time_range()`).
- [ ] `capture.rs`: `Capture` (all fields per DESIGN §4) + methods `warc_url`,
      `range_header`, `capture_key`, `looks_like_challenge`. Port + widen
      foundry's `CaptureLocator` logic; keep unit tests for `range_header`.
- [ ] `manifest.rs`: `Manifest`, `Sampling` enum (values only; sampling *logic*
      lands in M3). NDJSON (`to_ndjson`/`from_ndjson`) helpers.
- [ ] Unit tests: selector parsing round-trips, query builders, `range_header`,
      `capture_key` stability, `looks_like_challenge` heuristics.

### 1c. SURT — `src/surt.rs`

- [ ] `surt(url) -> String` and `surt_prefix_bounds(prefix) -> (String, String)`
      matching pywb/CC canonicalization (host reversal, default-port strip,
      lowercase, path normalization). Needed for dedup now, range scans in M3.
- [ ] Golden tests against known CC `url_surtkey` examples.

### 1d. Politeness — `src/polite.rs`

- [ ] `Politeness { max_rps, max_concurrency, retry, user_agent }`; per-host
      token buckets (`governor`) keyed by `index.` vs `data.` host; semaphore for
      concurrency; `RetryPolicy` (exp backoff + jitter, honor `Retry-After`,
      treat 503 as expected).
- [ ] Reject placeholder/empty `User-Agent` at build time (`Error::Config`).
- [ ] Tests: limiter admits N/interval; retry schedule is deterministic under a
      seeded/injected clock.

### 1e. Cache — `src/cache/`

- [ ] `trait Cache` (async) + `CacheKey`, `CacheHit`, `EntryMeta` (TTL, size).
- [ ] `FsCache` — CAS layout `ab/cd/<key>` under cache root, atomic write via
      temp+rename, TTL/size metadata sidecar; `stats()`, `gc()`.
- [ ] `MemoryCache` (moka, behind `moka` feature), `NoopCache`, `LayeredCache`.
- [ ] Three cache domains namespaced by key prefix: `registry:`, `index:`,
      `payload:` (payload keyed by `content_digest`).
- [ ] Tests: put/get/expiry/gc on a `tempfile` root; layered read-through.

### 1f. HTTP transport — `src/http.rs`

- [ ] Thin `reqwest` (rustls) wrapper that funnels every request through
      `Politeness` and the index-response cache; exposes `get_json`,
      `get_text`, `get_range` (for M2). Injectable for tests (trait +
      `MockTransport`).

### 1g. Registry — `src/registry.rs`

- [ ] `Registry` loads/caches `collinfo.json`; `CrawlInfo { id, name, cdx_api,
      from, to }`; `resolve(CrawlSelector) -> Vec<CrawlInfo>`; `refresh()`.
- [ ] Tests: resolve `LatestN`, `YearRange`, `Since` against a fixture
      `collinfo.json`.

### 1h. CDX backend — `src/index/`

- [ ] `cdx_compile.rs`: `UrlQuery` + `CrawlInfo` → CDX request URL (`url`,
      `matchType`, repeated `filter`, `collapse`, `fl`, `from`/`to`, `limit`,
      `output=json`). Port + extend foundry's `IndexQuery::endpoint`.
- [ ] `cdx_parse.rs`: parse CDXJ (JSON-lines) rows → `Capture`.
- [ ] `pagination.rs`: `showNumPages` probe → `IndexTooLarge` guard →
      page-by-page fetch; expose a `Stream<Item = Result<Capture>>` that pages
      lazily (never buffers the whole result). Multi-crawl = flatten per-crawl
      streams, ordered by crawl.
- [ ] `IndexClient { search(query) -> CaptureStream, count(query) -> Estimate }`.
- [ ] Tests: compile golden URLs; parse recorded CDXJ fixtures; paginate a
      2-page fixture; `count` reads `showNumPages`.

### 1i. Facade — `src/client.rs`

- [ ] `Ccdl` + `Ccdl::builder()` (`cache_dir`, `max_rps`, `max_concurrency`,
      `user_agent`, `registry_ttl`). Wires registry + cache + politeness +
      transport + `IndexClient`.
- [ ] Methods: `registry()`, `crawls()`, `search(query) -> CaptureStream`,
      `enumerate(pattern, match) -> EnumerateBuilder` (`.distinct_urls()`,
      `.crawls()`, `.status()`), `stats(query) -> Estimate`. Backend selection:
      M1 = CDX only (columnar `.bulk()` returns `Backend` unavailable until M3).
- [ ] `distinct_urls` dedups by `urlkey` across the stream with bounded memory
      note (hash set; document the ceiling, revisit with columnar in M3).

### 1j. CLI wiring — `ccdl-cli/src/`

- [ ] `output.rs`: `-o ndjson|jsonl|csv|table` writers; NDJSON is default and
      the agent contract (stable field order).
- [ ] Subcommands: `crawls [--refresh]`, `stats <pattern>`, `search <pattern>
      [--match][--status][--mime][--collapse][--crawls]`, `enumerate <pattern>
      --distinct`. `--dry-run` prints the compiled CDX URL(s).
- [ ] Exit codes: 0 ok, 2 usage, 3 rate-limited/exhausted, 4 too-large.
- [ ] `--budget` flags parsed (max-requests/bytes/pages) — enforced fully in M4,
      but `stats`/`--dry-run` honor them now.

**M1 acceptance:**

```bash
ccdl crawls --refresh
ccdl stats 'example.com/manufacturer/' --match prefix --crawls latest-4
ccdl enumerate 'example.com/manufacturer/' --match prefix \
     --crawls latest-4 --status 200 --distinct -o ndjson
```

runs end-to-end against live CC (behind `CCDL_LIVE=1` for the test), streams
without unbounded memory, and is polite (observable via rate-limit logs).

---

## Phase 2 (M2) — Fetch + WARC/HTTP parse + payload CAS

### 2a. WARC fetch/parse — `src/warc/`

- [ ] `fetch.rs`: `WarcClient::fetch(&Capture)` → range GET on `data.
      commoncrawl.org` (via cached transport), payload cached by `content_digest`
      (skip network on hit).
- [ ] `record.rs`: gunzip the single gzip member (`flate2`) → parse WARC headers
      → `WarcRecord`. Reuse the `warc` crate where it fits.
- [ ] `http.rs`: parse enclosed HTTP response (`httparse`) → `HttpCapture`
      (`status`, `headers`, raw body); `text()` handles charset +
      `content-encoding`; `looks_like_challenge()` upgraded to inspect body.
- [ ] Tests: golden WARC record fixtures (one 200 HTML, one redirect, one
      gzip-encoded body, one challenge page) → assert parsed fields + decoded
      text. Verify a range slice gunzips standalone.

### 2b. Facade + CLI

- [ ] `Ccdl::fetch(&Capture) -> WarcRecord`, `Ccdl::text(url) -> String` (resolve
      newest 200 capture, then fetch), `Ccdl::fetch_url(url, crawl)`.
- [ ] CLI: `fetch <url|@manifest.ndjson> [--crawl][--out dir/][--text]`,
      `text <url>`. `@manifest` streams captures from an NDJSON file and fetches
      each (respecting budget + politeness), writing `digest`-named files.

**M2 acceptance:** `ccdl fetch <url> --text` returns decoded page text from CC;
`ccdl manifest … | ccdl fetch @-` (piped) downloads a set with payload dedup
(second identical digest served from cache, verified in logs/tests).

---

## Phase 3 (M3) — Columnar backend + sampling + template extraction

### 3a. `manifest` + sampling — `src/model/sampling.rs`

- [ ] Implement `Sampling` reducers over a per-URL, time-ordered capture group:
      `All`, `UniqueDigest`, `FirstLast`, `FirstLastAndDigestChanges`,
      `AdaptiveChangePoint{max_per_url}`.
- [ ] `Ccdl::manifest(query).sample(..)` groups by `urlkey`, applies the reducer,
      returns `Manifest`. CLI `manifest <pattern> --sample …`.
- [ ] Tests: synthetic capture timelines → expected retained sets per policy.

### 3b. Pure-Rust columnar backend — `src/table/` (feature `table`)

- [ ] `store.rs`: `object_store` HTTP backend at `data.commoncrawl.org/cc-index/
      table/cc-main/warc/crawl=<id>/subset=warc/` (no AWS creds); opt-in S3
      backend (requester-pays).
- [ ] `compile.rs`: `UrlQuery` → predicate set: partition selection (`crawl`,
      `subset='warc'`), `url_host_name`/`url_host_tld` equality, `url_surtkey`
      range from `surt_prefix_bounds` (§1c), `fetch_status`/
      `content_mime_detected` filters; residual row filter for regex.
- [ ] `read.rs`: `arrow`/`parquet` reader with row-group + page pruning via
      column statistics; project only needed columns; map rows → `Capture`;
      expose a `CaptureStream` matching the CDX one. `TableClient` trait so CDX
      and columnar are interchangeable behind the facade.
- [ ] Facade: `Ccdl::search(query).bulk()` / auto-select columnar for large
      prefix/domain queries when `table` is enabled; else paginated CDX + warn.
- [ ] Tests: predicate compilation goldens; SURT range bounds; a small
      committed Parquet fixture (one crawl slice) → filtered rows. Live bulk test
      gated by `CCDL_LIVE=1`.

### 3c. Template extraction — `src/model/template.rs`

- [ ] `UrlTemplate::parse("host/manufacturer/{name}/{part}")`; compile to a
      match + capture; `bind(url) -> Option<Map<var,val>>`.
- [ ] `Ccdl::enumerate_template(&tmpl)` → enumerate the literal prefix, bind
      vars, group. CLI `enumerate --template '…/{name}/{part}'`.
- [ ] Tests: template parse, bind on matching/non-matching URLs, grouped output.

**M3 acceptance:** bulk cross-crawl enumeration of a domain via the columnar
backend returns the same `Capture` shape as CDX; `manifest --sample
first-last-digest` collapses an all-crawls timeline; template binding yields
`{name, part}` rows.

---

## Phase 4 (M4) — Discovery, budgets, Athena, foundry integration

- [ ] **Sitemap seed discovery** — `src/discover.rs`: fetch captured
      `robots.txt` / `sitemap.xml` (subset lookups) to seed enumeration of
      unknown URL schemes. CLI `sitemap <host>`.
- [ ] **Budgets** — enforce `--budget` (max requests/bytes/pages) across
      streams; `Error::BudgetExhausted`; deterministic partial results + resume
      token.
- [ ] **Athena adapter** — `src/athena.rs` (feature `athena`): submit SQL to the
      user's AWS account, poll, read result-set S3 → `Capture`. Additive; same
      `TableClient` trait.
- [ ] **Foundry integration** (DESIGN §9): foundry depends on `ccdl`; reduce
      `partfoundry-commoncrawl` to a thin adapter re-exporting `ccdl` types,
      keeping `ManufacturerDomainEpoch` + product policy; delete duplicated
      `CaptureLocator`/`IndexQuery`/`MatchType`/`HistoricalSamplingPolicy`; add
      an ADR recording foundry's first third-party dependency. *(Executed in the
      foundry repo, tracked here.)*

**M4 acceptance:** `ccdl sitemap <host>` seeds an enumeration; budgets cap a run
and emit a resume token; foundry builds against `ccdl` with the overlap removed.

---

## Cross-cutting: testing strategy

- **Hermetic default:** unit + fixture tests only; no network. `MockTransport`
  serves recorded CDXJ / JSON / WARC / Parquet fixtures under
  `crates/ccdl/tests/fixtures/`.
- **Golden tests:** compiled CDX URLs, SURT keys, columnar predicates, NDJSON
  output — assert against committed expected strings.
- **Live tests:** `#[ignore]` + `CCDL_LIVE=1`; a `just live` / `cargo test --
  --ignored` lane run manually or in a nightly CI job, kept small and polite.
- **CLI tests:** `assert_cmd` + `--dry-run` for deterministic command assertions.
- **Property tests** (optional, `proptest`): SURT round-trips, selector parsing.

## Cross-cutting: conventions

- Feature flags: `default=[index,warc,cache-fs,rustls]`, opt-in `table`,
  `athena`, `moka`. `--all-features` must build and clippy-clean.
- Every backend returns the same `CaptureStream = Stream<Item=Result<Capture>>`.
- `tracing` spans on every network op with host, crawl, cache hit/miss.
- Public items documented; `#![deny(missing_docs)]` on `ccdl` from M1.

## Risks / watch-list

- **CDX rate limits / 503s** — the reason politeness + backoff land in M1, not
  later. Keep live tests tiny.
- **Columnar memory** — stream row groups; never materialize a full crawl.
- **SURT correctness** — bugs here silently drop or over-match; golden tests are
  mandatory before M3 range scans.
- **`warc` crate fit** — if it can't parse single-record slices cleanly, fall
  back to a minimal in-crate WARC-header parser (headers are trivial; the value
  is in `httparse` + decoding).
- **Challenge pages** — validate `looks_like_challenge` against real Cloudflare
  captures in M2 fixtures so callers can filter them.

## Suggested sequencing

M1 is the critical path and independently useful. M2 is small and unlocks actual
byte retrieval. M3 is the largest (columnar + sampling + templates) and can be
parallelized: sampling/template work (pure, testable) alongside the columnar
reader. M4 is additive polish + the foundry cutover.
