## Context
Recent graph-build performance work proved wins with one-off manual benchmarks in task execution summaries. Those summaries are hard to compare after the task scrolls away, and the hot path (`ensure_fresh` before pack/search) can regress through pipeline, persistence, or cache changes. Criterion-style microbenchmarks would miss the dominant disk I/O and tree-sitter costs.

## Decision
Add `make bench` as a local end-to-end graph build benchmark. The driver lives in `orbit-knowledge`, calls `pipeline::run_build` directly, runs a cold full build plus a warm incremental no-op build against the repo root by default, and appends wall time/RSS/count metrics to `.orbit/state/scoreboard/graph_bench.json` capped at 200 records.

## Consequences
- Developers can compare graph build trends with machine/core context and git SHA preserved beside the metrics.
- No CI gate is introduced; shared-runner noise would make absolute thresholds misleading.
- The default corpus is maintenance-free because it is the repo itself, but timings and counts move as the repo grows. Use the scoreboard for trend-watching, not cross-machine normalization.
- Cost: regressions are advisory instead of blocking; maintainers must notice trend drift manually, and a repo-local corpus can hide performance cliffs that appear on larger or differently shaped workspaces.

---
