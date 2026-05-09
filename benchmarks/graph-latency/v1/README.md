# Graph Latency Benchmark v1

**Status: FROZEN** as of 2026-05-09. Records under `runs/`,
[`METHOD.md`](./METHOD.md), and [`RESULTS.md`](./RESULTS.md) are immutable per
[`../../CONVENTIONS.md`](../../CONVENTIONS.md) §Immutability. Factual
corrections go in `CORRECTIONS.md`; reinterpretation goes in v2 §Delta or a
shared compare doc.

- Method: [`METHOD.md`](./METHOD.md)
- Results: [`RESULTS.md`](./RESULTS.md)
- Run records: [`runs/`](./runs/)
- Next round: [`../v2/`](../v2/) (FROZEN — adds SQLite read fast paths for `graph.overview` / `graph.search` / `graph.show`).

## Headline

First baseline. orbit binary `cargo install`-built from `f6097e0a` immediately
before sweep, so the recorded `orbit_sha` reflects the actual binary's source
(via the cargo-install-immediately-before convention; see METHOD §Caveats).

TypeScript is 10-14× faster than Python and Java on identical operations.
Build-incremental is universally slower than build-cold (Python +46%, Java
+49%, TS +30%). 3 of 9 graph tools (`graph.deps`, `graph.implementors`,
`graph.history`) are inapplicable to non-Rust corpora and account for 88% of
the 51 failed cells. See [`RESULTS.md`](./RESULTS.md) for the full report.
