# Graph Latency Benchmark v1 Results

**No sweep yet — scaffold only.** This file is a placeholder. The first real
sweep is a follow-up task; once it lands, the sections below are populated
per the perf-RESULTS schema in [`../../CONVENTIONS.md`](../../CONVENTIONS.md).

The headers below match the required-section ordering for `kind: perf`
benchmarks. Do not delete or reorder them when filling this in — only replace
the placeholder body of each.

---

## 1. Frontmatter

_Placeholder. Populate at sweep time with: task ID, sweep date, sweep seed(s), scope (corpora × tools × phases × seeds), `orbit_sha`._

## 2. Headline

_Placeholder. 3–6 bullets summarizing the sweep. Lead with the most load-bearing finding (typically: which cells are over budget, where the regression or improvement lives)._

## 3. Primary latency table

_Placeholder. Corpus × tool × phase aggregate with columns `runs`, `p50_ms`, `p90_ms`, `p99_ms`, `budget_ms`, `over_budget`. Produced by `scripts/aggregate.py`._

## 4. Build-phase table

_Placeholder. Cold full build + warm incremental rebuild, per corpus tier. Columns `corpus`, `phase`, `wall_ms` (p50/p90/p99 over seeds), `rss_peak_mb`, `budget_ms`._

## 5. Host/environment disclosure

_Placeholder. CPU model, RAM, OS, and a statement of whether all primary-table rows came from one host or were aggregated. v1 rule: primary-table aggregation across hosts is not allowed._

## 6. Delta vs v(N-1)

v1 is the first frozen round; no prior version to diff.

## 7. Known caveats

_Placeholder. Cold-cache vs warm-cache, indexer parallelism, OSS-corpus representativeness — restate the caveats from `METHOD.md` and add anything specific to the actual sweep._

## 8. Recommendations

_Placeholder. Separate "change in the product" from "change in the next sweep."_
