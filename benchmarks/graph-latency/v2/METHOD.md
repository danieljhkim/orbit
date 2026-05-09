# Graph Latency Benchmark v2 Method

## Harness git SHA at freeze time

`<TBD-at-freeze>`. Set when v2 is frozen.

## Delta vs v1

`<TBD>`. v2 must record at least one measurement-affecting change vs v1 here
(fixture set diff, harness diff, system-under-test SHA pin diff, or
interpretive-frame diff) per `../../CONVENTIONS.md` §When to cut a new version.

Candidate v2 changes parked from v1's recommendations (decide before launching
the v2 sweep):
- Drop `graph.history` from the matrix (deprecated tool, no info value).
- Move `graph.deps` and `graph.implementors` to a Rust-only sub-matrix; add a
  `rust-medium` corpus.
- Tighten `queries.yaml` so seed=3 doesn't rotate to a `file:` selector for
  tools that reject it.
- Pin indexer parallelism via env var; record `host.parallelism_pin`.
- Bump the orbit binary SHA to a build that lands `graph update` incremental
  fix(es), to measure the build-incremental regression's resolution.

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
