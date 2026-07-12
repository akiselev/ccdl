---
name: ccdl
description: Query and download from Common Crawl using the `ccdl` CLI. Use when the user wants to find archived pages, enumerate URLs on a site, pull historical page content/HTML, sample how a page changed over time, or bulk-scan a domain from Common Crawl. Triggers on "common crawl", "ccdl", "archived version of", "what URLs did <site> have", "historical snapshot", "crawl the index for".
---

# Using ccdl (Common Crawl download & enumeration)

`ccdl` is a polite, streaming CLI over Common Crawl's index and WARC payloads.
Build it with `cargo build --release` (binary: `target/release/ccdl`); if a
`ccdl` is already on PATH, use that.

## Mental model

- **Crawl** = one monthly archive, e.g. `CC-MAIN-2026-30`. Queries span one or
  more crawls via `--crawls`.
- **Capture** = one archived fetch of one URL in one crawl (an index row): url,
  timestamp, HTTP status, mime, content digest, and the WARC `(file, offset,
  length)` locator. Default output is **NDJSON, one capture per line** — parse it
  as JSON Lines.
- Two index backends behind the same output shape: **CDX** (default, per-URL /
  small queries) and **columnar Parquet** (`--bulk`, for whole-domain scans).
  **WARC** fetch retrieves the actual captured bytes.

## Decide which command

| The user wants…                                   | Command |
| ------------------------------------------------- | ------- |
| list of monthly crawls                            | `ccdl crawls` |
| "how big is this query"                           | `ccdl stats <pattern>` |
| all captures matching a pattern                   | `ccdl search <pattern>` |
| the distinct URLs on a site/path                  | `ccdl enumerate <pattern> --distinct` |
| how a page changed over time                      | `ccdl manifest <pattern> --sample first-last-digest` |
| the text/HTML of a page                           | `ccdl text <url>` or `ccdl fetch <url> --out DIR` |
| download many pages                               | `ccdl enumerate … | ccdl fetch @- --out DIR` |
| structured fields from URLs                       | `ccdl enumerate <prefix> --template '…/{a}/{b}'` |
| seed URLs for an unknown site                     | `ccdl sitemap <host>` |
| a huge cross-crawl domain scan                    | `ccdl enumerate <pattern> --match domain --bulk --crawls all` |

## Core flags (apply broadly)

- `--match exact|prefix|host|domain` (default `prefix`). `prefix` = everything
  under a path; `host` = one host; `domain` = host + subdomains.
- `--crawls`: `latest` (default), `latest-N`, `all`, `2020..2024`,
  `since:2023-06-01`, `since:6mo`, or explicit `CC-MAIN-…,CC-MAIN-…`.
- `--from` / `--to`: filter by capture **timestamp** (vs `--crawls` which slices
  by crawl). Accepts `2021-06-15`, `2021-06`, `2021`, ISO/`YYYYMMDD`, and
  relative `3d` / `2w` / `6mo` / `1y` / `12h` / `yesterday` / `3 days ago`.
- `--newest` / `--oldest`: reduce a result to a single capture.
- `--status 200`, `--mime text/html`, `--collapse digest|urlkey`.
- `-o ndjson|csv|table` (default ndjson). `--max-rps` (default 2, keep it polite).
- `--dry-run` on `search`/`enumerate`: print the compiled CDX URL(s), run nothing.

## Recipes

Enumerate distinct product URLs, deduped, across recent crawls:
```bash
ccdl enumerate 'digikey.com/en/products/detail' \
  --match prefix --crawls latest-4 --status 200 --distinct -o ndjson
```

Newest archived text of one page:
```bash
ccdl text 'https://www.digikey.com/en/products/detail/3m/3341-5/1237486'
```

Download a set with payload dedup (identical digests served from cache):
```bash
ccdl enumerate 'example.com/' --match host --distinct --crawls latest-2 \
  | ccdl fetch @- --out ./pages/
```

Page-change timeline over all crawls:
```bash
ccdl manifest 'example.com/product/x' --crawls all --sample first-last-digest
```

Extract `{mfr, part, id}` from URLs:
```bash
ccdl enumerate 'digikey.com/en/products/detail' \
  --template 'digikey.com/en/products/detail/{mfr}/{part}/{id}'
```

Cap a big run and get a resume token (exit code 3 when hit):
```bash
ccdl enumerate 'example.com/' --match domain --distinct --budget-records 1000
```

## Interpreting output & exit codes

- NDJSON fields: `crawl, urlkey, url, timestamp, status, mime, mime_detected,
  languages, digest, length, offset, filename, redirect, truncated`.
- **An empty result is success (exit 0), not an error.**
- Exit codes: `0` ok · `2` usage/config · `3` rate-limited / budget exhausted ·
  `4` index too large.
- `status` 403/429/503 or `/cdn-cgi/` URLs are often **bot-challenge pages**, not
  real content — filter with `--status 200` when you want real pages.

## Guidance

- Start with `ccdl stats …` or `--dry-run` before a large run.
- Use CDX (default) for small/targeted queries; use `--bulk` (needs the `table`
  feature built in) only for whole-domain / many-crawl scans.
- Keep `--max-rps` modest — Common Crawl is a free, donated resource.
- A descriptive `--user-agent` is required; the default is fine for ad-hoc use.
- To resolve "the archived version of X near date D": `ccdl search '<url>'
  --match exact --crawls all --to '<D>' --newest`.
