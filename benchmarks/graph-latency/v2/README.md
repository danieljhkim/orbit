# Graph Latency Benchmark v2

**Status: FROZEN** as of 2026-05-09. Records under `runs/`,
[`METHOD.md`](./METHOD.md), and [`RESULTS.md`](./RESULTS.md) are immutable per
[`../../CONVENTIONS.md`](../../CONVENTIONS.md) §Immutability. Factual
corrections go in `CORRECTIONS.md`; reinterpretation goes in v3 §Delta or a
shared compare doc.

- Method: [`METHOD.md`](./METHOD.md)
- Results: [`RESULTS.md`](./RESULTS.md)
- Run records: [`runs/`](./runs/)

## Headline

orbit binary `cargo install`-built from `bacf6309` immediately before sweep,
so the recorded `orbit_sha` reflects the actual binary's source. v2's only
change vs v1 is the system-under-test pin (post-`T20260509-70..-74` SQLite
read fast paths); fixtures are byte-identical.

`graph.show`, `graph.search`, and `graph.overview` got SQL fast paths and
deliver: 95-98% latency reduction on Python/Java and 67-79% on TypeScript.
The cross-language gap on these three tools collapsed from ~10× (v1) to ~1×
(v2). Build-cold stays inside its <10% cap on Python (+8%) and faster on
Java (-14%, run-to-run noise dominating). Build-incremental is still
pathologically slower than build-cold across all three languages (+24-45%) —
T-70..-74 only touched read paths. The 51 structural failure cells from v1
reproduce identically. See [`RESULTS.md`](./RESULTS.md) for the full report.

## Next round

Round 3 lives at [`../v3/`](../v3/) (LIVING). v3's parked candidate changes
combine the v1-era list with v2-derived findings: drop `graph.history`,
tighten `queries.yaml` seed=3, pin indexer parallelism, flush OS page caches
before `build-cold`, add a Rust corpus, embed build SHA in `orbit --version`,
and the new top-of-list candidate of wiring SQLite fast paths for
`graph.callers` and `graph.refs`.
