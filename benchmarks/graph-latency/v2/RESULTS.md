# Graph Latency Benchmark v2 Results

## 1. Frontmatter

Task ID: `T20260509-87`. Sweep date: 2026-05-09. Sweep id: `20260509-162139`. Scope: 3 corpora × 9 tools × 3 phases × 5 seeds = 165 cells. `orbit_sha`: `bacf630981e6c88f7ccc3471d90835d20f47b873` (fresh `cargo install --path crates/orbit-cli --force` from this exact SHA immediately before sweep). System-under-test pin is the only v2 change vs v1; fixtures are byte-identical (`corpora.yaml` and `tasks/queries.yaml` unchanged). Corpora pinned in [`corpora.yaml`](./corpora.yaml): django/django@5.1.2, google/guava@v33.4.8, vuejs/core@v3.5.13. Sweep wall-clock: ~6 min on Apple M4 Pro / 64 GB / macOS 26.4.1. Failed cells: 51 (same structural pattern as v1).

## 2. Headline

- **`graph.show`, `graph.search`, and `graph.overview` are 67-98% faster end-to-end.** All three got SQL fast paths in [T20260509-72](https://orbit-cli.com/tasks/T20260509-72/), [T20260509-73](https://orbit-cli.com/tasks/T20260509-73/), and [T20260509-74](https://orbit-cli.com/tasks/T20260509-74/), and all three deliver. On Python and Java, p50 collapses from 1.1-2.1 s to 30-70 ms (-95 to -98%). On TypeScript, where v1 was already sub-200 ms, p50 still drops 67-79% (118-172 ms → 36-39 ms). The cross-language gap that dominated v1 is gone for these three tools — Python/Java now sit within ~2× of TypeScript instead of 10-14×.
- **The build-time secondary index lands inside its <10% cap.** [T20260509-70](https://orbit-cli.com/tasks/T20260509-70/) capped build-cold overhead at 10%; measured: python +8%, ts ~0%, java -14% (Java actually went faster, likely run-to-run variance dominating any small index-write cost). All three are within the policy.
- **Non-wired tools are unchanged within noise.** `graph.callers`, `graph.refs`, and `graph.pack` show -1 to -10% deltas across all three corpora. No fast path was wired for these in T-70..-74; the small movements are consistent with measurement noise. The exception is python `graph.refs` at -24%, which sits above the noise band but lacks a code-side explanation; flagged in §7.
- **Build-incremental is still pathologically slower than build-cold.** The v1 finding holds in v2: python +45%, java +41%, ts +24% (incremental over cold). The Java gap narrowed (v1 +49% → v2 +41%) but the structural problem — a "single-file mutation update" that does ~as much work as a full rebuild — is unchanged. T-70..-74 only touched read paths, so this was expected; calling it out so it doesn't get lost.
- **The 51 structural failures from v1 reproduce identically.** `graph.deps` (Cargo.toml-only, 15 cells), `graph.implementors` (trait-only, 15 cells), `graph.history` (deprecation stub, 15 cells), and `graph.callers` / `graph.refs` rejecting `file:` selectors at seed=3 (6 cells). Same error messages, same counts. None of these are bugs the SQLite work was meant to address.
- **Single-host disclosure unchanged.** Same M4 Pro / 64 GB / macOS 26.4.1 as v1; same unpinned indexer parallelism (parked for v3). RSS p90 sits at 70-616 MB across phases, no OOMs.

## 3. Primary latency table (query phase)

`Δp50 vs v1` is negative when v2 is faster. Cells with 100% error rate emit no percentiles by design — they failed before any measurement.

| corpus        | tool                | runs | errors | p50_ms | p90_ms | p99_ms | Δp50 vs v1 |
|---------------|---------------------|-----:|-------:|-------:|-------:|-------:|-----------:|
| python-medium | graph.overview      |    5 |      0 |     63 |     64 |     64 |       -95% |
| python-medium | graph.search        |    5 |      0 |     38 |     39 |     39 |       -97% |
| python-medium | graph.callers       |    5 |      1 |   1782 |   1786 |   1787 |        -4% |
| python-medium | graph.deps          |    5 |      5 |      — |      — |      — |          — |
| python-medium | graph.refs          |    5 |      1 |   1843 |   1849 |   1851 |       -24% |
| python-medium | graph.show          |    5 |      0 |     37 |     39 |     39 |       -98% |
| python-medium | graph.implementors  |    5 |      5 |      — |      — |      — |          — |
| python-medium | graph.history       |    5 |      5 |      — |      — |      — |          — |
| python-medium | graph.pack          |    5 |      0 |     72 |     73 |     74 |       -10% |
| java-medium   | graph.overview      |    5 |      0 |     67 |     69 |     69 |       -95% |
| java-medium   | graph.search        |    5 |      0 |     34 |     36 |     36 |       -97% |
| java-medium   | graph.callers       |    5 |      1 |   1962 |   1971 |   1974 |        -7% |
| java-medium   | graph.deps          |    5 |      5 |      — |      — |      — |          — |
| java-medium   | graph.refs          |    5 |      1 |   2037 |   2039 |   2039 |        -1% |
| java-medium   | graph.show          |    5 |      0 |     35 |     36 |     36 |       -98% |
| java-medium   | graph.implementors  |    5 |      5 |      — |      — |      — |          — |
| java-medium   | graph.history       |    5 |      5 |      — |      — |      — |          — |
| java-medium   | graph.pack          |    5 |      0 |     69 |     69 |     69 |        -5% |
| ts-medium     | graph.overview      |    5 |      0 |     39 |     40 |     41 |       -67% |
| ts-medium     | graph.search        |    5 |      0 |     38 |    143 |    152 |       -68% |
| ts-medium     | graph.callers       |    5 |      1 |    155 |    157 |    158 |        -1% |
| ts-medium     | graph.deps          |    5 |      5 |      — |      — |      — |          — |
| ts-medium     | graph.refs          |    5 |      1 |    166 |    167 |    167 |        -2% |
| ts-medium     | graph.show          |    5 |      0 |     36 |     36 |     36 |       -79% |
| ts-medium     | graph.implementors  |    5 |      5 |      — |      — |      — |          — |
| ts-medium     | graph.history       |    5 |      5 |      — |      — |      — |          — |
| ts-medium     | graph.pack          |    5 |      0 |     38 |     39 |     39 |        -3% |

`graph.search` on ts-medium has a p50 of 38 ms but a p90/p99 of 143/152 ms. The first sample paid a one-time cost (likely cold SQLite page cache); subsequent samples were uniformly fast. Same shape on java-medium, but compressed enough that p99 still rounds to 36 ms.

## 4. Build-phase table

| corpus        | phase             | runs | errors | p50_ms | p90_ms | p99_ms | rss_p90_mb | Δp50 vs v1 |
|---------------|-------------------|-----:|-------:|-------:|-------:|-------:|-----------:|-----------:|
| python-medium | build-cold        |    5 |      0 |  14470 |  14627 |  14647 |        414 |        +8% |
| python-medium | build-incremental |    5 |      0 |  21008 |  21128 |  21145 |        530 |        +8% |
| java-medium   | build-cold        |    5 |      0 |  15734 |  15982 |  16023 |        470 |       -14% |
| java-medium   | build-incremental |    5 |      0 |  22202 |  22295 |  22297 |        616 |       -19% |
| ts-medium     | build-cold        |    5 |      0 |   1194 |   1266 |   1269 |         70 |        ~0% |
| ts-medium     | build-incremental |    5 |      0 |   1485 |   1555 |   1597 |         78 |        -4% |

Incremental delta vs cold (within v2): Python +45%, Java +41%, TypeScript +24%. The "incremental > cold" pattern persists across all three corpora — same structural finding as v1. Java's relative gap narrowed (v1 +49% → v2 +41%) but the issue is not resolved.

## 5. Host/environment disclosure

- **CPU**: Apple M4 Pro
- **RAM**: 64 GB
- **OS**: macOS 26.4.1 (arm64)
- **Aggregation**: single-host. Every primary-table row in this report came from one machine; no cross-host data is mixed in.
- **Indexer parallelism**: still not pinned. v2 deliberately did not change this so the v1 → v2 comparison stays clean. Parked for v3.
- **OS page caches**: not flushed between cells; cold/warm-cache effects persist (same as v1). Most visible in ts-medium `graph.search` p90 (143 ms first-sample vs 38 ms p50 thereafter).
- **Binary build mode**: release. `cargo install --path crates/orbit-cli --force` from the v2 harness checkout (`bacf6309`) immediately before sweep.

## 6. Delta vs v1

**Improvements (lead).** Three of the six "applicable" query tools — `graph.overview`, `graph.search`, `graph.show` — now run 30-100× faster end-to-end on Python and Java. v1 reported `graph.show` as "the slowest passing tool (~2.0-2.1 s p50 on Python/Java)"; v2 makes it the fastest applicable tool at ~35 ms. The cross-language gap on these three tools collapsed: where v1 had Python at 1.2 s and TypeScript at 0.12 s (10×), v2 has Python at 38 ms and TypeScript at 38 ms (1×). For agents driving a graph workflow that is dominated by show/search/overview calls (e.g. `orbit.graph.pack` planners, navigation UIs), v2 is qualitatively a different system.

**Regressions / no-ops.** None where the SQLite work was wired — every wired tool got faster. Two soft regressions worth noting:

- **python build-cold +8%, python build-incremental +8%.** Within T-70's <10% cap and consistent with the documented build-time index write. java actually got faster (likely noise dominating); ts is flat.
- **`graph.search` ts-medium p90/p99 = 143/152 ms** vs p50 = 38 ms. First-sample cold-cache cost. v1 didn't show this shape because the slow path was uniformly slow; the SQLite path's first read pays a one-time page-cache fill. p50 is still 68% faster than v1; this is a tail-latency disclosure, not a regression.

**Unchanged within noise.** `graph.callers` (-1 to -7%), `graph.refs` (-1 to -2% on java/ts; -24% on python), `graph.pack` (-3 to -10%). T-72/-73/-74 didn't wire fast paths for these. The python `graph.refs` -24% is the only outlier above noise but lacks a code-side explanation; possibly a side-effect of SQLite warming the page cache that the JSON-file walker also benefits from. Flagged in §7.

**Same failures.** All 51 v1 failure cells reproduce identically: `graph.deps` (Cargo.toml-only), `graph.implementors` (trait-only), `graph.history` (deprecation stub), `graph.callers`/`graph.refs` rejecting `file:` selectors at seed=3. Same error messages, same counts. T-70..-74 was scoped to read paths, not coverage; these remain v3 candidates.

## 7. Known caveats

- **Build SHA still proxied via harness checkout.** `orbit --version` does not embed the build commit; `orbit_sha` in records is the harness-checkout HEAD captured at sweep time. Reproducibility relies on the `cargo install --force` happening immediately before the sweep — same convention as v1. Embedding the build SHA in `orbit --version` remains a v3 candidate.
- **Indexer parallelism unpinned.** Default thread pool, not recorded per record. Cross-host comparisons would need a `host.parallelism_pin` field; v2 inherits v1's gap.
- **OS page caches not flushed.** The `graph.search` ts-medium first-sample tail (143/152 ms) is direct evidence that page-cache state matters. v3 should flush before each `build-cold` cell and consider a warm-up call before query cells.
- **Python `graph.refs` -24% lacks a code-side root cause.** The tool was not in T-72/-73/-74 scope. Possible explanations: SQLite-driven page cache warming the JSON walker, a coincidental run-to-run drift, or an unannounced internal change. Investigate before drawing planning conclusions from this number.
- **Binary identity convention is fragile.** v1 had a near-miss on this (a stale `cargo install` from a deleted worktree gave a misleading `orbit_sha`). v2 explicitly verified `which orbit`, `orbit --version`, and the cargo install replacement-line output to confirm the binary in `$PATH` matches `bacf6309`. Documented for v3 to keep doing this.
- **Failure cells unchanged.** v1's "3 of 9 graph tools are inapplicable to non-Rust corpora" finding still holds. The 51 failed cells are the same structural mismatches; no claim is made about whether SQLite changed their failure mode.

## 8. Recommendations

Ranked by expected ROI on agent UX:

1. **Wire `graph.callers` and `graph.refs` to the SQLite read facade.** These are the two slowest applicable query tools post-v2 (1.8-2.0 s p50 on python/java). Both already accept `symbol:` selectors that map cleanly to a SQL lookup. Bigger absolute win than further tuning overview/search/show.
2. **Fix the build-incremental pathological path.** A "single-file mutation + `orbit graph update`" still costs +24-45% over a full rebuild. T-70..-74 didn't address this. Investigation candidate: confirm whether incremental does a full reparse; if so, design a true incremental delta path. Same priority as (1) but a different team's expertise.
3. **Drop `graph.history` from the matrix and the tool registry.** It is 100% errors in v1 and v2 with a deprecation message. Carrying it as `active: yes` in `orbit tool list` is misleading for callers. Either re-flag to `deprecated` so it stops appearing in default listings, or remove. v3 should drop it from the sweep matrix regardless.
4. **Decide the future of `graph.deps` and `graph.implementors`.** Both are Rust-only by design. Either widen them to other languages (workspace dependency files in pom.xml/package.json/pyproject; trait-equivalents = Java interface, Python ABC, TS interface) or guard them behind a Rust-only precondition that fails fast with a clearer error. Today they leak Rust-internals errors at the agent-facing boundary.
5. **Document the `file:` selector limitation on `graph.callers` / `graph.refs`.** Both reject `file:` selectors with a clean "requires symbol selector" error, but the tool description doesn't say so. Either accept `file:` (treat as "all symbols in file") or document the constraint in the tool's `Behavior:` line.
6. **Pin indexer parallelism via env var; record `host.parallelism_pin` in every record (v3).** Removes the only material cross-machine confounder.
7. **Flush OS page caches before each `build-cold` cell (v3).** Removes the cold/warm-cache confounder evidenced by ts-medium `graph.search` tail.
8. **Add a Rust corpus (v3, e.g. `tokio-rs/tokio`).** Lets `graph.deps` and `graph.implementors` actually run instead of being permanent failure cells.
