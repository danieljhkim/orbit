# Graph Latency Benchmark v1

**Status: FROZEN** as of 2026-05-09. The records under `runs/`, the
[`METHOD.md`](./METHOD.md), and the [`RESULTS.md`](./RESULTS.md) are immutable
per [`../../CONVENTIONS.md`](../../CONVENTIONS.md) §Immutability. Factual
corrections go into a `CORRECTIONS.md` here; reinterpretation goes into
`v2/METHOD.md` §Delta or a shared `COMPARE-v1-vs-v2.md`.

- Method: [`METHOD.md`](./METHOD.md)
- Results: [`RESULTS.md`](./RESULTS.md)
- Run records: [`runs/`](./runs/)

## Headline

TypeScript is 10-14× faster than Python and Java on identical operations.
Build-incremental is universally slower than build-cold (Python +44%, Java
+34%, TS +13%). 3 of 9 graph tools (`graph.deps`, `graph.implementors`,
`graph.history`) are inapplicable to non-Rust corpora and account for 88%
of the 51 failed cells. See [`RESULTS.md`](./RESULTS.md) for the full report.
