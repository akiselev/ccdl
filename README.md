# ccdl

A polite, streaming Rust library and CLI for enumerating and downloading
captures from [Common Crawl](https://commoncrawl.org) — the free, monthly,
petabyte-scale web archive.

`ccdl` turns "what did this site look like, and where are the bytes" into a few
commands. It speaks Common Crawl's three access surfaces:

- the **CDX index** (per-URL lookups, the default),
- the **columnar Parquet index** (`table` feature, for bulk domain/prefix scans),
- **WARC payloads** (byte-range fetch of the actual captured bytes).

See [DESIGN.md](./DESIGN.md) for architecture and [PLAN.md](./PLAN.md) for the
build plan.

---

## Install

```bash
cargo build --release
# binary at target/release/ccdl
```

The library crate is `ccdl`; the binary crate is `ccdl-cli` (binary name `ccdl`).

## Quick start

```bash
# List available monthly crawls
ccdl crawls

# How big is this query? (index page estimate)
ccdl stats 'digikey.com/en/products/detail' --match prefix --crawls latest-4

# Enumerate distinct product URLs across the last 4 crawls, as NDJSON
ccdl enumerate 'digikey.com/en/products/detail' \
    --match prefix --crawls latest-4 --status 200 --distinct

# Pull the newest captured version of a page as text
ccdl text 'https://www.digikey.com/en/products/detail/3m/3341-5/1237486'
```

Everything streams: enumerating a domain across many crawls never buffers the
whole result set in memory, and every request is rate-limited per host.

---

## Concepts

**Crawl.** Common Crawl publishes one crawl per month, e.g. `CC-MAIN-2026-30`.
Queries span one or more crawls (see `--crawls` below).

**Capture.** One archived fetch of one URL in one crawl — a row from the index.
It carries the URL, timestamp, HTTP status, MIME, content digest, and the
`(warc_filename, offset, length)` needed to fetch the bytes. NDJSON output is one
capture per line with stable field order — the agent-friendly contract.

**Match type** (`--match`): how the pattern matches index URLs.

| value    | meaning                              |
| -------- | ------------------------------------ |
| `exact`  | exactly this URL                     |
| `prefix` | any URL under this path prefix (default) |
| `host`   | any URL on this host                 |
| `domain` | this host and its subdomains         |

**Output** (`-o/--output`): `ndjson` (default), `csv`, `table`.

---

## Selecting crawls and time

### `--crawls <selector>`

Which crawls a query spans:

| selector            | meaning                                             |
| ------------------- | --------------------------------------------------- |
| `latest`            | the single newest crawl (default)                   |
| `latest-4`          | the 4 newest crawls                                 |
| `all`               | every known crawl (~100)                            |
| `2020..2024`        | crawls in that year range (inclusive)               |
| `since:2023-06-01`  | crawls whose coverage ends on/after that date       |
| `since:6mo`         | …relative form (see below)                          |
| `CC-MAIN-2026-30,CC-MAIN-2026-25` | explicit crawl ids                    |

`since:` compares against each crawl's **real coverage-end date** (from the
registry), not just the year.

### `--from` / `--to` (capture timestamp filter)

Where `--crawls` slices by *crawl*, `--from`/`--to` slice by the **capture's
actual timestamp** — compiled to the CDX `from`/`to` filter. Either side may be
omitted.

```bash
ccdl search 'example.com/' --match host --from '2 years ago' --to today
```

### Date/time formats (accepted by `--from`, `--to`, and `since:`)

**Absolute:** `2021`, `2021-06`, `2021-06-15`, `2021-06-15T08:30:00Z`,
and CDX-style `20210615` / `20210615083000`.

**Relative** (resolved against now): `now`, `today`, `yesterday`, and
`<n><unit>` / `<n> <unit> ago` where unit is one of
`h`/`hour(s)`, `d`/`day(s)`, `w`/`week(s)`, `mo`/`month(s)`, `y`/`year(s)`:

```
3d          3 days ago       12h        2w        6mo        1y
```

### `--newest` / `--oldest`

Reduce any `search`/`enumerate` result to the single newest (or oldest) matching
capture by timestamp:

```bash
ccdl search 'https://example.com/page' --match exact --crawls all --newest
```

---

## Commands

### `crawls [--refresh]`
List available crawls (`--refresh` re-fetches the registry).

### `stats <pattern> [--match]`
Estimate query size (index page count) without downloading rows.

### `search <pattern> [--match --status --mime --collapse --newest --oldest --dry-run]`
Stream matching captures. `--dry-run` prints the compiled CDX URL(s) instead of
running — useful for debugging and for feeding other tools.

### `enumerate <pattern> [--match --status --distinct --budget-records --template --bulk --dry-run]`
Enumerate URLs. `--distinct` dedups by url key. `--budget-records N` stops after
N records and prints a **resume token** (exit code 3). `--template` binds a URL
template (below). `--bulk` uses the columnar backend (below).

```bash
# Deduped enumeration with a 1000-record budget
ccdl enumerate 'example.com/' --match domain --distinct --budget-records 1000
```

### `manifest <pattern> [--match --sample]`
Enumerate, then **sample** per URL over time. Policies (`--sample`):

| policy               | keeps                                        |
| -------------------- | -------------------------------------------- |
| `all`                | every capture                                |
| `unique-digest`      | one per distinct content hash                |
| `first-last`         | earliest and latest only                     |
| `first-last-digest`  | first, last, and every content change (default) |
| `adaptive`           | change-points, bounded per URL               |

```bash
# How did this product page change over the last year?
ccdl manifest 'example.com/product/x' --crawls all --sample first-last-digest
```

### `fetch <url | @manifest.ndjson> [--out DIR --text]`
Download captured bytes. A bare URL fetches its newest 200. `@file.ndjson` (or
`@-` for stdin) streams captures and fetches each, writing digest-named files;
identical digests are served from cache (payload dedup).

```bash
ccdl enumerate 'example.com/' --match host --distinct \
  | ccdl fetch @- --out ./pages/
```

### `text <url>`
Resolve the newest 200 capture of a URL and print its decoded text (handles
gzip + charset).

### `sitemap <host>`
Discover sitemap seed URLs from the host's **archived** `robots.txt` /
`sitemap.xml`, to seed enumeration of unknown URL schemes.

### URL templates
Extract structured variables from enumerated URLs:

```bash
ccdl enumerate 'digikey.com/en/products/detail' \
  --template 'digikey.com/en/products/detail/{mfr}/{part}/{id}'
# {"url":"…/3m/3341-5/1237486","vars":{"mfr":"3m","part":"3341-5","id":"1237486"}}
```

---

## Global flags

| flag              | default | meaning                                  |
| ----------------- | ------- | ---------------------------------------- |
| `--crawls`        | `latest`| crawl selector (above)                   |
| `--from` / `--to` | –       | capture timestamp range (above)          |
| `--newest`/`--oldest` | –   | reduce to a single capture               |
| `-o/--output`     | `ndjson`| `ndjson` / `csv` / `table`               |
| `--max-rps`       | `2`     | max requests/sec per host bucket         |
| `--user-agent`    | `ccdl/…`| descriptive UA (required; empty rejected)|
| `--cache-dir`     | XDG cache | cache root                              |

## Exit codes

| code | meaning                              |
| ---- | ------------------------------------ |
| 0    | ok (including an empty result set)   |
| 2    | usage / config error                 |
| 3    | rate-limited or budget exhausted     |
| 4    | index result too large               |

An **empty enumeration is success**, never an error.

---

## The columnar backend (`--bulk`, `table` feature)

For large prefix/domain scans, CDX's paginated API is slow and gets throttled.
The `table` feature reads Common Crawl's **columnar Parquet index** directly from
`data.commoncrawl.org` (no AWS credentials), pruning row groups by `url_surtkey`
range statistics so you scan only the relevant chunks.

```bash
ccdl enumerate 'example.com/' --match domain --bulk --crawls all
```

Prefer CDX for small queries; prefer `--bulk` when pulling a whole
domain/prefix across many crawls. Both return the same capture shape.

---

## Library usage

A complete, runnable version of the snippet below lives at
[`crates/ccdl/examples/enumerate.rs`](./crates/ccdl/examples/enumerate.rs):

```bash
cargo run --example enumerate -- 'en.wikipedia.org/wiki/'
```

```rust
use ccdl::prelude::*;
use ccdl::model::crawl::CrawlSelector;
use futures::StreamExt;

# async fn run() -> ccdl::Result<()> {
let ccdl = Ccdl::builder()
    .user_agent("myapp/1.0 (+https://example.com)")
    .max_rps(2)
    .build()?;

let query = UrlQuery::prefix("en.wikipedia.org/wiki/")
    .status(200)
    .crawls(CrawlSelector::LatestN(4));

let mut stream = ccdl.search(query).await?;
while let Some(capture) = stream.next().await {
    let c = capture?;
    println!("{} {}", c.timestamp, c.url);
}
# Ok(())
# }
```

### Cargo features

| feature      | default | provides                                    |
| ------------ | ------- | ------------------------------------------- |
| `index`      | ✅      | CDX backend                                 |
| `warc`       | ✅      | WARC fetch/parse, payload CAS               |
| `cache-fs`   | ✅      | filesystem cache                            |
| `rustls`     | ✅      | rustls TLS                                  |
| `table`      | –       | columnar Parquet backend                    |
| `athena`     | –       | Athena SQL adapter                          |
| `moka`       | –       | in-memory cache tier                        |

---

## Being a good citizen

Common Crawl is free and donated. `ccdl` is polite by construction: per-host
rate limiting, bounded concurrency, exponential backoff honoring `Retry-After`,
and a required descriptive `User-Agent`. Keep `--max-rps` modest and prefer the
columnar backend over hammering the CDX API for bulk work.
