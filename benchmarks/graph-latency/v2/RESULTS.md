# Graph Latency Benchmark v2 Results

## 1. Frontmatter

Task ID: `T20260509-63`. Sweep date: 2026-05-09. Sweep id: `v2-fresh-binary`. Scope: same as v1 — 3 corpora × 9 tools × 3 phases × 5 seeds = 165 cells. `orbit_sha`: `f6097e0a119631728f76e09f3d82c73867cf1684` (fresh `cargo install --path crates/orbit-cli --force` immediately before sweep). Corpora unchanged from v1: django/django@5.1.2, google/guava@v33.4.8, vuejs/core@v3.5.13. Sweep wall-clock: 607 s (~10 min, +10% vs v1's 552 s) on the same host as v1 (Apple M4 Pro / 64 GB / macOS 26.4.1). Single-host aggregation.

Single variable changed vs v1: orbit binary SHA. See [`METHOD.md`](./METHOD.md) §Delta vs v1 for the full diff plus the v1 binary-identity caveat.

## 2. Headline

- **v1→v2 mostly preserves v1's qualitative findings.** TypeScript still 10-14× faster than Python and Java on identical operations; build-incremental still universally slower than build-cold; the same 51 structural failures (3 Rust-only/deprecated tools × all corpora × N seeds + seed=3 file:-selector rotation × 2 tools × 3 corpora) reproduce exactly. The pathologies surfaced in v1 are not transient.
- **Two material regressions vs v1.** Python `graph.refs` p50 jumped **+32%** (1843 → 2426 ms); Java build-incremental p50 jumped **+21%** (22517 → 27268 ms). TS build-incremental also regressed **+14%** (1362 → 1553 ms). These are the cells worth investigating before v3.
- **The "incremental slower than cold" gap widened in v2.** Java incremental was +34% over cold in v1; in v2 it's +49%. TS incremental was +13% over cold in v1; in v2 it's +30%. The incremental codepath is moving in the wrong direction.
- **Most other cells drifted within ±10%.** Of the 18 (corpus × tool) query rows that successfully complete in both versions, 13 changed by ≤7% — within the range plausibly explained by host noise + binary build mode. No cells improved by more than 3%; the fresh binary brought no net wins on the measured surface.
- **The v1 binary-identity caveat dominates the v1→v2 delta interpretation.** v1's recorded `orbit_sha=1b4a9be8` was a harness-checkout proxy, not the actual binary's build SHA — the v1 binary in PATH was a stale `orbit-cli v0.1.0` install predating the recorded SHA. v2's `orbit_sha=f6097e0a` is accurate (cargo-installed immediately before sweep from that exact tree). The delta therefore measures "stale v0.1.0 → fresh v0.3.1 release-mode," not a clean source-SHA delta. v3+ deltas will be reliable; v1→v2 is best read as "establishing the first real baseline" rather than "this code change caused this latency change."

## 3. Primary latency table (query phase)

| corpus        | tool                | runs | errors | p50_ms | p90_ms | p99_ms | Δp50 vs v1 |
|---------------|---------------------|-----:|-------:|-------:|-------:|-------:|-----------:|
| python-medium | graph.overview      |    5 |      0 |   1159 |   1164 |   1165 |        +0% |
| python-medium | graph.search        |    5 |      0 |   1191 |   1256 |   1293 |        +3% |
| python-medium | graph.callers       |    5 |      1 |   1862 |   1888 |   1893 |        +7% |
| python-medium | graph.deps          |    5 |      5 |      — |      — |      — |          — |
| python-medium | graph.refs          |    5 |      1 |   2426 |   2860 |   2997 |   **+32%** |
| python-medium | graph.show          |    5 |      0 |   2034 |   2322 |   2441 |        +7% |
| python-medium | graph.implementors  |    5 |      5 |      — |      — |      — |          — |
| python-medium | graph.history       |    5 |      5 |      — |      — |      — |          — |
| python-medium | graph.pack          |    5 |      0 |     80 |     88 |     93 |       +11% |
| java-medium   | graph.overview      |    5 |      0 |   1268 |   1355 |   1363 |        −1% |
| java-medium   | graph.search        |    5 |      0 |   1316 |   1354 |   1357 |        +2% |
| java-medium   | graph.callers       |    5 |      1 |   2102 |   2186 |   2204 |        +7% |
| java-medium   | graph.deps          |    5 |      5 |      — |      — |      — |          — |
| java-medium   | graph.refs          |    5 |      1 |   2063 |   2115 |   2122 |        +2% |
| java-medium   | graph.show          |    5 |      0 |   2134 |   2180 |   2185 |        +2% |
| java-medium   | graph.implementors  |    5 |      5 |      — |      — |      — |          — |
| java-medium   | graph.history       |    5 |      5 |      — |      — |      — |          — |
| java-medium   | graph.pack          |    5 |      0 |     73 |     83 |     89 |        +4% |
| ts-medium     | graph.overview      |    5 |      0 |    118 |    127 |    132 |        −1% |
| ts-medium     | graph.search        |    5 |      0 |    119 |    125 |    126 |        +0% |
| ts-medium     | graph.callers       |    5 |      1 |    157 |    163 |    164 |        −3% |
| ts-medium     | graph.deps          |    5 |      5 |      — |      — |      — |          — |
| ts-medium     | graph.refs          |    5 |      1 |    170 |    176 |    176 |        +1% |
| ts-medium     | graph.show          |    5 |      0 |    172 |    188 |    197 |        −1% |
| ts-medium     | graph.implementors  |    5 |      5 |      — |      — |      — |          — |
| ts-medium     | graph.history       |    5 |      5 |      — |      — |      — |          — |
| ts-medium     | graph.pack          |    5 |      0 |     39 |     40 |     40 |        +0% |

Cells with 100% error rate are unchanged from v1 — same Rust-only / deprecated / file-selector mismatches. v3 will drop or guard them.

## 4. Build-phase table

| corpus        | phase             | runs | errors | p50_ms | p90_ms | p99_ms | rss_p90_mb | Δp50 vs v1 |
|---------------|-------------------|-----:|-------:|-------:|-------:|-------:|-----------:|-----------:|
| python-medium | build-cold        |    5 |      0 |  13379 |  15364 |  16452 |        386 |        −2% |
| python-medium | build-incremental |    5 |      0 |  19516 |  19623 |  19632 |        523 |        −1% |
| java-medium   | build-cold        |    5 |      0 |  18254 |  20532 |  20695 |        444 |        +9% |
| java-medium   | build-incremental |    5 |      0 |  27268 |  30165 |  30398 |        624 |   **+21%** |
| ts-medium     | build-cold        |    5 |      0 |   1195 |   1281 |   1324 |         66 |        −1% |
| ts-medium     | build-incremental |    5 |      0 |   1553 |   1650 |   1668 |         73 |       +14% |

Incremental-vs-cold gap by language (v2): Python +46%, Java +49%, TypeScript +30%. All three languages are worse off than they were in v1 (+44%, +34%, +13%). The incremental codepath is the dominant build-phase regression.

## 5. Host/environment disclosure

- **CPU**: Apple M4 Pro (same machine as v1)
- **RAM**: 64 GB
- **OS**: macOS 26.4.1 (arm64)
- **Aggregation**: single-host. No cross-host data mixed in.
- **Indexer parallelism**: still not pinned. v3 should pin via env var and record `host.parallelism_pin`; without it, "Java +9%/+21%" could conceivably be partially explained by parallelism drift, though the consistent direction across cold and incremental argues against it.
- **Binary build mode**: v2 used `cargo install --path crates/orbit-cli --force` which produces a release-mode binary. v1's binary is suspected (not confirmed) to have been an older `cargo install` of v0.1.0; build mode unknown.

## 6. Delta vs v1

Highlights from §3 and §4:

| Cell                            | v1 p50 | v2 p50 | Δ        | Severity |
|---------------------------------|-------:|-------:|---------:|----------|
| python-medium graph.refs        |   1843 |   2426 | **+32%** | high     |
| java-medium build-incremental   |  22517 |  27268 | **+21%** | high     |
| ts-medium build-incremental     |   1362 |   1553 |     +14% | medium   |
| python-medium graph.pack        |     72 |     80 |     +11% | medium   |
| java-medium build-cold          |  16771 |  18254 |      +9% | medium   |
| python/java graph.callers       |   1742 |   1862 |      +7% | low      |
| python-medium graph.show        |   1905 |   2034 |      +7% | low      |
| All other cells                 |      — |      — |   ±0-7% | noise    |

No cell improved by more than 3%. Read against the v1 binary-identity caveat in §2: the regressions are real signal that v2's binary is slower than v1's was, but the cause is not pinpointable from one delta — it's the bundled effect of (probably) v0.1.0 → v0.3.1 plus debug-vs-release plus any other build-config drift.

## 7. Known caveats

- **v1 binary identity is uncertain** — see §2 headline and `METHOD.md` §Delta. Most important caveat for reading this report.
- **Same structural failure pattern as v1.** `graph.history` deprecated, `graph.deps` Rust-only, `graph.implementors` Rust-trait-only, `graph.callers`/`graph.refs` reject `file:` selectors. v3 should drop or guard these per v1's recommendation #1.
- **Cold-cache effects unpinned** — same caveat as v1. Page-cache state can shift `build-cold` numbers by ~2x. The v1 and v2 sweeps ran on the same host on the same day; the page cache state at v2 time was shaped by v1's runs (the corpora are the same paths). This *helps* the build-cold v1→v2 comparison (similar page-cache state), but it's incidental — v3 should sync+drop page caches before each `build-cold` cell.
- **Single-language corpus per language.** Same as v1. The `+32%` python-medium graph.refs regression could be django-specific or python-parser-wide; we can't tell with one corpus. v3 candidate: add a second python corpus (flask, fastapi) to disambiguate.
- **Subprocess startup cost is bundled into wall_ms** — same as v1. For the small TS query cells (~120 ms p50) this is a meaningful share. v3 candidate: an MCP-server-based harness if sub-100 ms targets become budgets.
- **Build-incremental mutation is a single appended line.** Same minimum-delta floor as v1.
- **Sweep wall-clock grew +10%** (552 s → 607 s). Roughly consistent with the per-cell regressions; not a separate signal.

## 8. Recommendations

### Change in the product

1. **Investigate Python `graph.refs` (+32%) and Java build-incremental (+21%) before v3.** These are the two cells where v2's binary is materially slower than v1's. If the cause is a recent commit, it's a regression worth reverting or fixing. If it's the v0.1.0→v0.3.1 jump bundling many changes, the regression is permanent and the new baseline is just slower — in which case the perf-recovery work is its own thread.
2. **Embed the build's git SHA in the orbit binary** (e.g. via `vergen` or a `build.rs` writing into a `static const`). `orbit --version` should print it. Once available, the harness records the binary's true SHA instead of the harness-checkout proxy, and the v1 caveat goes away forever. This is a one-time fix that protects every future round.
3. **Fix `graph update` so incremental is faster than cold.** v1 said this; v2 says it more loudly. Three languages × two rounds × six observations all show incremental >> cold. The pattern is not subtle and not transient.
4. **Address the v1 recommendations that didn't move:** drop/inactivate `graph.history`, document the symbol-only constraint on `graph.callers`/`graph.refs`, make `graph.deps` graceful on non-Rust corpora, close the Python/Java vs TS parser gap.

### Change in the next sweep (v3)

1. **Drop `graph.history` from the matrix** and **fix the seed=3 `file:`-selector rotation** so `graph.callers`/`graph.refs` only see `symbol:` selectors. Removes 36 of the 51 failing cells; the cleaner matrix improves signal-to-noise.
2. **Pin indexer parallelism.** Set `RAYON_NUM_THREADS=8` (or whatever the matching value is) via env var, record it as `host.parallelism_pin` in every record. Without this, host noise has a free axis to vary on.
3. **Sync + drop page caches before each `build-cold` cell.** `sudo purge` on macOS or `echo 3 > /proc/sys/vm/drop_caches` on Linux. Removes the cold/warm-cache confounder.
4. **Add a second corpus per language** — the +32% python `graph.refs` regression begs the question "is this django-specific?" A second python corpus (flask or fastapi) decides it.
5. **Once the orbit-side `--version` SHA embedding lands, drop the `orbit_sha` proxy in run.py** and read it from the binary directly.
