# Graph Latency Benchmark v2

**Status: LIVING.** Round 2 is in progress. Inputs (`corpora.yaml`,
`tasks/queries.yaml`) start as a copy of v1's; the measurement change that
justifies cutting v2 will be recorded in `METHOD.md §Delta vs v1` once it
lands.

- Method: [`METHOD.md`](./METHOD.md) (placeholder until measurement variable is fixed)
- Results: [`RESULTS.md`](./RESULTS.md) (placeholder until first v2 sweep)
- Run records: [`runs/`](./runs/) (gitignored until v2 freeze)

The v1 frozen baseline is at [`../v1/`](../v1/). v2 must change at least one
of (fixtures, harness, system-under-test pin, interpretive frame) per
[`../../CONVENTIONS.md`](../../CONVENTIONS.md) §When to cut a new version. A
re-run on identical inputs is seed expansion, not a version cut.
