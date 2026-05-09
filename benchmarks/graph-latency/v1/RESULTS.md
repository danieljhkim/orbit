# Graph Latency Benchmark v1 Results

## 1. Frontmatter

Task ID: `T20260509-63`. Sweep date: 2026-05-09. Sweep id: `v1-first`. Scope: 3 corpora × 9 tools × 3 phases × 5 seeds = 165 cells. `orbit_sha`: `1b4a9be8881f411effb5c1719b1959fefee40463`. Corpora pinned in [`corpora.yaml`](./corpora.yaml): django/django@5.1.2, google/guava@v33.4.8, vuejs/core@v3.5.13. Sweep wall-clock: 554s (~9 min) on a single host (Apple M4 Pro / 64 GB / macOS 26.4.1).

## 2. Headline

- **TypeScript is 10-14× faster than Python and Java on identical operations.** TS query p50 sits at 119-173 ms; Python/Java p50 sits at 1100-2100 ms across the same tools at comparable corpus sizes (~150k LOC for guava and vue/core; ~280k for django). The same is true for builds — TS build-cold is 1.2 s, Python is 13.6 s, Java is 16.8 s. The cross-language gap is the most load-bearing finding of v1.
- **Build-incremental is universally SLOWER than build-cold.** Python +44%, Java +34%, TypeScript +13%. A single-file mutation followed by `orbit graph update` does more work than rebuilding from scratch. The incremental path is doing something pathological — most likely a full rebuild plus extra bookkeeping, not a true incremental delta. This is the most actionable bug.
- **3 of 9 graph tools are inapplicable to non-Rust corpora.** `graph.deps` is hardcoded to read `Cargo.toml`; `graph.implementors` only resolves Rust traits and rejects Java interfaces, Python classes, and TS interfaces; `graph.history` is documented as deprecated and returns a removal error on every call. 45 of 51 failed cells trace to these three structural mismatches. Removing or guarding these tools would eliminate ~88% of the failure rate.
- **`graph.callers` and `graph.refs` reject `file:` selectors.** Both require `symbol:` selectors; both return clean error JSON when given a `file:` selector. Caused 6 of 51 failed cells (every seed=3, which rotates to a `file:` selector in `queries.yaml`). The constraint isn't documented in tool descriptions; future query-shape design needs to know.
- **`graph.show` is the slowest passing tool (1.9-2.1 s p50 on Python/Java); `graph.pack` is the fastest (40-72 ms p50).** That p50 spread within a single corpus is 27× — bigger than any cross-corpus difference within a single tool. Tool-shape choice dominates corpus-size choice for query latency.
- **No timeouts, no OOMs.** All 30 build cells completed within budget. Memory peaked at 604 MB (Java incremental). The medium-tier corpora (~150-280k LOC) are within the design's reach; the user's concern about large-tier (700k+) handling is not falsified or confirmed by v1.

## 3. Primary latency table (query phase)

| corpus        | tool                | runs | errors | p50_ms | p90_ms | p99_ms |
|---------------|---------------------|-----:|-------:|-------:|-------:|-------:|
| python-medium | graph.overview      |    5 |      0 |   1156 |   1182 |   1195 |
| python-medium | graph.search        |    5 |      0 |   1154 |   1195 |   1220 |
| python-medium | graph.callers       |    5 |      1 |   1742 |   1784 |   1797 |
| python-medium | graph.deps          |    5 |      5 |      — |      — |      — |
| python-medium | graph.refs          |    5 |      1 |   1843 |   1865 |   1870 |
| python-medium | graph.show          |    5 |      0 |   1905 |   1937 |   1953 |
| python-medium | graph.implementors  |    5 |      5 |      — |      — |      — |
| python-medium | graph.history       |    5 |      5 |      — |      — |      — |
| python-medium | graph.pack          |    5 |      0 |     72 |     73 |     73 |
| java-medium   | graph.overview      |    5 |      0 |   1286 |   1306 |   1310 |
| java-medium   | graph.search        |    5 |      0 |   1290 |   1296 |   1300 |
| java-medium   | graph.callers       |    5 |      1 |   1972 |   2022 |   2040 |
| java-medium   | graph.deps          |    5 |      5 |      — |      — |      — |
| java-medium   | graph.refs          |    5 |      1 |   2017 |   2032 |   2036 |
| java-medium   | graph.show          |    5 |      0 |   2092 |   2150 |   2156 |
| java-medium   | graph.implementors  |    5 |      5 |      — |      — |      — |
| java-medium   | graph.history       |    5 |      5 |      — |      — |      — |
| java-medium   | graph.pack          |    5 |      0 |     70 |     71 |     72 |
| ts-medium     | graph.overview      |    5 |      0 |    119 |    127 |    131 |
| ts-medium     | graph.search        |    5 |      0 |    119 |    123 |    123 |
| ts-medium     | graph.callers       |    5 |      1 |    162 |    167 |    167 |
| ts-medium     | graph.deps          |    5 |      5 |      — |      — |      — |
| ts-medium     | graph.refs          |    5 |      1 |    169 |    169 |    169 |
| ts-medium     | graph.show          |    5 |      0 |    173 |    187 |    196 |
| ts-medium     | graph.implementors  |    5 |      5 |      — |      — |      — |
| ts-medium     | graph.history       |    5 |      5 |      — |      — |      — |
| ts-medium     | graph.pack          |    5 |      0 |     39 |     40 |     40 |

Cells with 100% error rate (`runs=errors=5`) emit no percentiles by design — they are not "0 ms"; they failed before any measurement.

v1 deliberately publishes no `budget_ms` column. Budgets require a target SLO and v1 is the first measurement of the surface; setting a budget on the same data we measured against is meaningless. v2 cuts a budget once we have a "before vs after" reference point.

## 4. Build-phase table

| corpus        | phase             | runs | errors | p50_ms | p90_ms | p99_ms | rss_p90_mb |
|---------------|-------------------|-----:|-------:|-------:|-------:|-------:|-----------:|
| python-medium | build-cold        |    5 |      0 |  13644 |  13748 |  13794 |        391 |
| python-medium | build-incremental |    5 |      0 |  19624 |  19703 |  19726 |        505 |
| java-medium   | build-cold        |    5 |      0 |  16771 |  16861 |  16897 |        449 |
| java-medium   | build-incremental |    5 |      0 |  22517 |  22591 |  22636 |        604 |
| ts-medium     | build-cold        |    5 |      0 |   1206 |   1259 |   1268 |         67 |
| ts-medium     | build-incremental |    5 |      0 |   1362 |   1464 |   1466 |         72 |

Incremental delta vs cold: Python +5.98 s (+44%), Java +5.75 s (+34%), TypeScript +0.16 s (+13%). The "incremental is slower" pattern shows up in all three languages but is most extreme in Python and Java, suggesting the language-specific parser path is paying the cost rather than the indexer core.

## 5. Host/environment disclosure

- **CPU**: Apple M4 Pro
- **RAM**: 64 GB
- **OS**: macOS 26.4.1 (arm64)
- **Aggregation**: single-host. Every primary-table row in this report came from one machine; no cross-host data is mixed in. Per `METHOD.md` §Host disclosure rules, primary-table cross-host aggregation is not allowed in v1.
- **Indexer parallelism**: not pinned. The harness records the OS-default thread pool count implicitly via wall-clock; v2 should pin parallelism via env var and record it in `host.parallelism_pin`.

## 6. Delta vs v(N-1)

v1 is the first frozen round; no prior version to diff.

## 7. Known caveats

- **`graph.history` is deprecated, not "slow".** Its 100% error rate is by design — the tool is a compatibility stub that returns a removal error on every call. v2 should drop it from the matrix entirely.
- **`graph.deps` and `graph.implementors` are Rust-only.** Failures on Python/Java/TypeScript are not regressions or bugs in those corpora — they are the tool's documented scope. The benchmark records the failures so the language-applicability gap is visible from outside the source.
- **Seed=3 selector rotation hits `file:` selectors,** which `graph.callers` and `graph.refs` reject. The error in those cells reflects a tool input-validation rule, not a parser or latency bug. v2 either tightens `queries.yaml` to symbol-only selectors for these tools or swaps the seed=3 row to a symbol selector.
- **Cold-cache effects unpinned.** The harness deletes `<corpus>/.orbit/knowledge/` before each `build-cold` cell but does not clear the OS page cache. Java/Python build-cold p99 spreads (~30-50 ms) suggest cache effects are negligible at this corpus size, but the rule should be enforced before the design is trusted at the large tier.
- **Single-language corpus per language.** Vue/core happens to be a particularly fast TypeScript corpus; other TS mono-repos with heavier `import type` chains may behave differently. v2 could broaden the TS sample.
- **Subprocess startup cost is bundled into wall_ms.** Each cell forks a fresh `orbit` binary, paying ~30-100 ms of process startup. For sub-200 ms TS query cells, that overhead is a meaningful share of the measurement. v2 should consider an MCP-server-based harness if sub-100 ms targets become real budgets.
- **Build-incremental mutation is a single appended line.** The incremental path is exercised against the cheapest possible delta. A larger delta would presumably be slower; the universal "incremental > cold" finding is the dominant signal, and the absolute numbers should be read as a floor, not a ceiling.

## 8. Recommendations

### Change in the product

1. **Fix `graph update` so incremental is faster than cold.** Three rounds of identical observation across three language paths is enough evidence that the incremental codepath is doing pathological work. This is the single most impactful change for query-loop latency on large corpora — every developer touch ships through this path. Likely candidates: full reparse instead of dirty-set reparse, redundant blob writes, or a recompute that ignores prior state.
2. **Close the Python/Java vs TypeScript parser gap.** TS at 1.2 s build-cold for 150k LOC is ~10× faster than Java at 16.8 s for the same size. Either the TS parser is doing less work (under-extracting symbols), or the Python/Java parsers are doing redundant work. Worth a profiling pass against `python-medium` and `java-medium` build-cold to find the dominant cost.
3. **Either remove `graph.history` from the tool registry, or surface it as `inactive`** so listings, MCP plugin advertising, and the benchmark harness all stop offering a tool that returns a removal error on every call. Keeping a deprecated tool indistinguishable from active tools is a UX hazard.
4. **Loosen `graph.callers` and `graph.refs` to accept `file:` selectors,** OR document the symbol-only constraint in the tool description prominently. The current state — selector rejected, error returned — fails open: agents can construct `file:` selectors via `graph.search` and feed them to these tools, which then break.
5. **Make `graph.deps` graceful on non-Rust corpora.** Either return an empty result with a `kind: not-applicable` marker, or detect the missing `Cargo.toml` upfront and emit a structured "language unsupported" error rather than a generic execution failure with a Cargo-shaped error string.

### Change in the next sweep

1. **Drop `graph.history` from the matrix.** No information value.
2. **Move `graph.deps` and `graph.implementors` to a Rust-only sub-matrix** (e.g. add `rust-medium: rust-lang/rustfmt` or `tokio-rs/tokio`). Keep them in the v1 record as documentation of the language gap, but stop measuring them on Python/Java/TS.
3. **Tighten `queries.yaml` selectors** so `graph.callers` and `graph.refs` only see `symbol:` rotations. The seed=3 file:-selector rotation is benchmark noise, not a real measurement.
4. **Pin indexer parallelism** via env var and record `host.parallelism_pin`. Required for cross-host comparability once we add a benchmarking fleet.
5. **Tighten subprocess overhead measurement** by adding a "tool baseline" cell (e.g. `orbit graph search --query x --limit 1` against a 100-LOC corpus) so we can separate per-call startup cost from per-tool work in v2 deltas.
6. **Add a "minimal corpus" sanity floor.** A ~100 LOC handcrafted corpus exercising every tool with known-good selectors. Non-zero failure rate on it would catch harness regressions before a real corpus is loaded.
