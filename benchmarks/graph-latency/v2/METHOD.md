# Graph Latency Benchmark v2 Method

## Harness git SHA at freeze time

`bacf630981e6c88f7ccc3471d90835d20f47b873` (`agent-main` HEAD on 2026-05-09 at sweep time). This is the harness-checkout commit; it is also the source SHA the orbit binary was built from via `cargo install --path crates/orbit-cli --force` immediately before the sweep — same convention as v1. The convention is fragile (a stale `cargo install` from another checkout would silently shadow it); v2 explicitly re-verified `which orbit`, `orbit --version`, and the cargo install replacement-line output before sweeping. Embedding the build SHA in `orbit --version` remains a v3 candidate.

## Delta vs v1

**System-under-test pin only.** Fixtures are byte-identical to v1: `corpora.yaml` and `tasks/queries.yaml` are unchanged. The orbit binary changed from `f6097e0a` (pre-SQLite read paths) to `bacf6309` (post `T20260509-70..-74`):

- T20260509-70 — Build SQLite secondary index during graph write (additive write path; <10% build-cost cap).
- T20260509-71 — SQLite read facade with version check + graceful fallback.
- T20260509-72 — `graph.overview` summary fast path.
- T20260509-73 — `graph.search` exact-name and prefix fast paths.
- T20260509-74 — `graph.show` by selector fast path.

No harness, fixture, or interpretive-frame changes. The other v1 candidate changes (drop `graph.history`, tighten `queries.yaml` seed=3, pin indexer parallelism, sync + drop OS page caches, second corpus per language, embed build SHA in `orbit --version`, add a Rust corpus) were deliberately held back so the v1 → v2 comparison axis stays single-variable. They carry forward to v3.

## Corpus list

Inherits v1's three corpora unchanged:

- `python-medium` — django/django @ `c499184f` (tag 5.1.2).
- `java-medium` — google/guava @ `f06690fa` (tag v33.4.8).
- `ts-medium` — vuejs/core @ `6eb29d34` (tag v3.5.13).

`corpora.yaml` and `tasks/queries.yaml` are byte-identical to v1's frozen versions.

## In-scope tools

Same 9 `orbit.graph.*` tools as v1: `overview`, `search`, `callers`, `deps`, `refs`, `show`, `implementors`, `history`, `pack`. v2 deliberately keeps the full set so the structural-failure baseline stays visible; v3 will drop `graph.history` since it is a deprecation stub with 100% errors in both rounds.

## Phases

Same as v1: `build-cold`, `build-incremental`, `query`. `build-cold` measures `orbit graph build` from scratch; `build-incremental` appends a marker line to a corpus-specific `mutation_path` and runs `orbit graph update`; `query` runs each in-scope tool once per seed against the most recently built graph.

## Per-cell record schema

Same as v1 (see [`../v1/METHOD.md`](../v1/METHOD.md) §Per-cell record schema). Fields: `corpus`, `corpus_sha`, `error`, `host` (`cpu` / `os` / `ram_gb`), `orbit_sha`, `phase`, `query_shape`, `result_count`, `result_size_bytes`, `rss_peak_mb`, `seed`, `tool`, `wall_ms`. Schema breaks require a new round.

## Host disclosure rules

Same as v1: single-host primary table; cross-host data only in appendix. v2 was run on a single host (Apple M4 Pro / 64 GB / macOS 26.4.1) — same host as v1, so the `Δp50 vs v1` column in [`RESULTS.md`](./RESULTS.md) compares like-for-like.

## Reproduction command

```bash
# 1. Pin to the v2 SHA and rebuild the binary.
git checkout bacf6309
cargo install --path crates/orbit-cli --force

# 2. Fetch corpora (idempotent if the cache survives).
GRAPH_LATENCY_VERSION=v2 make -C benchmarks graph-latency-fetch

# 3. Run the sweep.
GRAPH_LATENCY_VERSION=v2 make -C benchmarks graph-latency-sweep

# 4. Aggregate with v1 baseline (the make target does not expose --baseline,
#    so call the script directly).
python3 benchmarks/graph-latency/scripts/aggregate.py \
  --runs benchmarks/graph-latency/v2/runs \
  --baseline benchmarks/graph-latency/v1/runs
```
