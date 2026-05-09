# Graph Latency Benchmark v3 Method

## Harness git SHA at freeze time

`<TBD-at-freeze>`. Set when v3 is frozen.

## Delta vs v2

`<TBD>`. v3 must record at least one measurement-affecting change vs v2 here
per `../../CONVENTIONS.md` §When to cut a new version. See `README.md` for
the candidate list.

## Corpus list

Inherits v2's three corpora unchanged at scaffold time. `corpora.yaml` and
`tasks/queries.yaml` are copies of v2's frozen versions; edit either to record
a v3 fixture-set change.

## In-scope tools

Same as v2 (all 9 `orbit.graph.*`) at scaffold time. Likely v3 change:
drop `graph.history`.

## Phases

Same as v2: `build-cold`, `build-incremental`, `query`.

## Per-cell record schema

Same as v1/v2 (see [`../v1/METHOD.md`](../v1/METHOD.md) §Per-cell record
schema). Schema breaks require a new round.

## Host disclosure rules

Same as v1/v2: single-host primary table; cross-host data only in appendix.

## Reproduction command

```bash
GRAPH_LATENCY_VERSION=v3 make -C benchmarks graph-latency-fetch
GRAPH_LATENCY_VERSION=v3 make -C benchmarks graph-latency-sweep
GRAPH_LATENCY_VERSION=v3 make -C benchmarks graph-latency-aggregate
```
