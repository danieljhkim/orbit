# Graph Latency Benchmark v3

**Status: LIVING.** Round 3 is in progress. Inputs (`corpora.yaml`,
`tasks/queries.yaml`) start as a copy of v2's; the measurement change that
justifies cutting v3 will be recorded in `METHOD.md §Delta vs v2` once it
lands.

- Method: [`METHOD.md`](./METHOD.md)
- Results: [`RESULTS.md`](./RESULTS.md) (placeholder until first v3 sweep)
- Run records: [`runs/`](./runs/) (gitignored until v3 freeze)

The v2 frozen baseline is at [`../v2/`](../v2/). v3 must change at least one
of (fixtures, harness, system-under-test pin, interpretive frame) per
[`../../CONVENTIONS.md`](../../CONVENTIONS.md) §When to cut a new version.

## Candidate v3 changes (parked from v2's recommendations)

- **Drop `graph.history` from the matrix** (deprecated; 100% errors in v1+v2; no info value).
- **Tighten `queries.yaml` seed=3** so `graph.callers`/`graph.refs` only see `symbol:` selectors. Removes 6 noise cells.
- **Pin indexer parallelism** via env var; record `host.parallelism_pin` in every record.
- **Sync + drop OS page caches** before each `build-cold` cell to remove the cold/warm-cache confounder.
- **Add a second corpus per language** (e.g. flask alongside django) to disambiguate corpus-specific regressions like v2's python `graph.refs` +32%.
- **Investigate or revert** the Python `graph.refs` (+32%) and Java build-incremental (+21%) regressions before running v3 — a v3 sweep against the same binary that produced v2 reproduces the regressions but doesn't explain them.
- **Embed build SHA in `orbit --version`** so the harness can record the binary's true source instead of the harness-checkout proxy. Once available, drop the proxy in run.py.
