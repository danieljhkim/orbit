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

## Candidate v3 changes

Top of the list — derived from v2 findings (see [`../v2/RESULTS.md`](../v2/RESULTS.md) §8):

- **Wire SQLite fast paths for `graph.callers` and `graph.refs`.** Now the two slowest applicable query tools post-v2 (1.8-2.0 s p50 on python/java). Both already accept `symbol:` selectors that map cleanly to a SQL lookup. Bigger absolute win than further tuning overview/search/show.
- **Fix the build-incremental pathological path.** Single-file mutation + `orbit graph update` still costs +24-45% over a full rebuild. Likely a full reparse rather than a true incremental delta. Investigate before further tuning the read paths.
- **Drop `graph.history` from the matrix** (deprecated; 100% errors in v1 and v2; no info value). Either re-flag to `deprecated` so it stops appearing in default `orbit tool list`, or remove. Either way, drop from sweep.

Carried forward from v1 (still parked):

- **Tighten `queries.yaml` seed=3** so `graph.callers`/`graph.refs` only see `symbol:` selectors. Removes 6 noise cells.
- **Pin indexer parallelism** via env var; record `host.parallelism_pin` in every record.
- **Sync + drop OS page caches** before each `build-cold` cell to remove the cold/warm-cache confounder. v2 surfaced direct evidence this matters (ts-medium `graph.search` p90/p99 = 143/152 ms vs p50 = 38 ms).
- **Add a second corpus per language** (e.g. flask alongside django) to disambiguate corpus-specific findings.
- **Embed build SHA in `orbit --version`** so the harness records the binary's true source instead of the harness-checkout proxy. v1 and v2 both relied on the fragile "cargo install immediately before sweep" convention.
- **Add a Rust corpus** (e.g. `tokio-rs/tokio` or `rust-lang/rustfmt`) so `graph.deps` and `graph.implementors` actually run instead of being permanent failure cells.
- **Decide the future of `graph.deps` and `graph.implementors`.** Rust-only by design. Either widen them to other languages (workspace dependency files; trait-equivalents = Java interface, Python ABC, TS interface) or guard them behind a Rust-only precondition that fails fast with a clearer error.
- **Document the `file:` selector limitation on `graph.callers` / `graph.refs`.** Both reject `file:` selectors but tool descriptions don't say so.
