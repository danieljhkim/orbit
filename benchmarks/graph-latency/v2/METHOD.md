# Graph Latency Benchmark v2 Method

## Harness git SHA at freeze time

`f6097e0a119631728f76e09f3d82c73867cf1684` — the orbit binary that produced
every record under `v2/runs/` was `cargo install`-built from this SHA
immediately before the sweep. Reproducing v2 requires the harness sources at
this SHA AND a release-mode orbit binary built from it.

## Delta vs v1

**Single variable changed: orbit binary SHA.**

| Aspect              | v1                                          | v2                                          |
|---------------------|---------------------------------------------|---------------------------------------------|
| Corpora             | django 5.1.2, guava v33.4.8, vue/core v3.5.13 | unchanged                                   |
| `queries.yaml`      | unchanged                                   | unchanged                                   |
| Tools in matrix     | all 9 `orbit.graph.*`                       | unchanged                                   |
| Seeds (N)           | 5                                           | 5                                           |
| Phases              | build-cold + build-incremental + query      | unchanged                                   |
| Host                | Apple M4 Pro / 64 GB / macOS 26.4.1         | unchanged                                   |
| **orbit_sha**       | `1b4a9be8881f411effb5c1719b1959fefee40463`  | `f6097e0a119631728f76e09f3d82c73867cf1684`  |
| **orbit binary**    | `cargo install` v0.1.0 (older, debug-mode-suspected) | `cargo install --path crates/orbit-cli` v0.3.1 release |

Holding every other variable constant lets the v1→v2 delta speak unambiguously
to "orbit code change". v3 will fold in matrix cleanup (drop `graph.history`,
tighten seed=3 selectors, optionally add a Rust corpus) — bundling those into
v2 would confound the delta.

**Caveat on v1's recorded `orbit_sha`.** v1 records claim
`orbit_sha=1b4a9be8...` but the orbit binary actually in `~/.cargo/bin/orbit`
at v1 measurement time was a stale `cargo install` of v0.1.0, predating the
`1b4a9be8` source. The harness's `orbit_sha` getter reads the harness checkout
HEAD (a proxy), not the binary's embedded build SHA — `orbit --version` does
not currently expose a build SHA, so the binary's true source is best-effort.
v2's `orbit_sha=f6097e0a` is accurate because v2 was preceded by an immediate
`cargo install --path crates/orbit-cli --force` from that exact SHA. The v1→v2
delta therefore reflects a binary change that is real but not precisely
`1b4a9be8 → f6097e0a`; the lower bound of the change is "stale v0.1.0 →
fresh v0.3.1".

A future v(N) should fix this: orbit-cli `--version` should embed the build's
git SHA (via `build.rs` or `vergen`) so the harness can record the binary's
true source. Tracked as a v3 candidate change.

## Corpus list

Inherits v1's three corpora unchanged at scaffold time. `corpora.yaml` and
`tasks/queries.yaml` are copies of v1's frozen versions; edit either to record
a v2 fixture-set change.

## In-scope tools

Same as v1 (all 9 `orbit.graph.*`) at scaffold time. v2 may narrow this if
`graph.history` is dropped per the candidate change above.

## Phases

Same as v1: `build-cold`, `build-incremental`, `query`.

## Per-cell record schema

Same as v1 (see [`../v1/METHOD.md`](../v1/METHOD.md) §Per-cell record schema).
A schema break is a hard reason to cut a new version; v2 inherits v1's schema
unchanged unless this section says otherwise at freeze.

## Host disclosure rules

Same as v1: single-host primary table; cross-host data only in appendix.

## Reproduction command

```bash
GRAPH_LATENCY_VERSION=v2 make -C benchmarks graph-latency-fetch
GRAPH_LATENCY_VERSION=v2 make -C benchmarks graph-latency-sweep
GRAPH_LATENCY_VERSION=v2 make -C benchmarks graph-latency-aggregate
```
