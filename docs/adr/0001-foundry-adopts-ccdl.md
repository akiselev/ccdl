# ADR 0001 — foundry adopts `ccdl` as its first third-party dependency

Status: Proposed (executed in the foundry repo; tracked here per PLAN.md M4).

## Context

`partfoundry-commoncrawl` grew its own Common Crawl primitives —
`CaptureLocator`, `IndexQuery`, `MatchType`, and `HistoricalSamplingPolicy` —
before `ccdl` existed. `ccdl` now provides a superset of these as a standalone,
tested, polite library (DESIGN §9, decision 2).

## Decision

foundry depends on `ccdl` and retires the overlapping primitives:

| foundry type                | replacement                               |
| --------------------------- | ----------------------------------------- |
| `CaptureLocator`            | `ccdl::model::Capture` (superset)         |
| `IndexQuery`                | `ccdl::model::UrlQuery` + CDX compiler    |
| `MatchType`                 | `ccdl::model::MatchType`                  |
| `HistoricalSamplingPolicy`  | `ccdl::model::Sampling` + `sampling::sample` |

`partfoundry-commoncrawl` becomes a thin adapter that re-exports `ccdl` types and
keeps only foundry-specific policy: `ManufacturerDomainEpoch` and the product
URL policy.

## Consequences

- First third-party dependency for foundry; `ccdl` forbids `unsafe`, matching
  foundry's lints, and is async-only (no blocking facade yet).
- Duplicated canonicalization/pagination/sampling logic is deleted, removing a
  class of drift bugs (e.g. SURT correctness lives in one place with golden
  tests).
- foundry can inject its own `ccdl::cache::Cache` implementation to back onto its
  artifact store (DESIGN §11, open item 4).
