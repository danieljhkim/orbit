# Graph Latency Benchmark

kind: perf

Measures wall-clock latency of `orbit.graph.*` MCP tool invocations on large polyglot mono-repos. No agent in the loop — direct tool calls only. Answers the question: *for a given orbit revision, how fast is each graph tool on a real-world large corpus, and where are we over budget?*

This benchmark is the durable signal for graph-tool perf work. Improvements and regressions land as new frozen rounds with a `Delta vs v(N-1)` table; vibes-based "feels faster now" claims are out of scope.

For agent-tool *budget* questions (does an agent reach for the graph, does it save tokens), see the closed [`benchmarks/graph/`](../graph/) series — that's a different benchmark with a different `kind`.

## Scope

Two phases per (corpus × tool) cell:

- **Build phase** — cold full index of the corpus, then warm incremental rebuild after a controlled mutation (rename / content edit / move).
- **Query phase** — N seeded calls per tool per corpus, distribution reported as p50/p90/p99.

Tools in scope (one cell per): `orbit.graph.overview`, `search`, `callers`, `deps`, `refs`, `show`, `implementors`, `history`, `pack` — all 9 graph MCP tools.

## Corpus matrix

Three tiers across three languages — Python, Java, TypeScript. Concrete `<org>/<repo>@<sha>` pins are recorded in each round's `vN/METHOD.md`. Fixtures live outside the repo to keep our git footprint small — `scripts/fetch.sh` clones into `~/.cache/orbit-bench/<corpus>` on first run.

| Tier   | Target LOC   | Rationale                                                       |
|--------|--------------|-----------------------------------------------------------------|
| small  | ~10k         | sanity floor; sub-100ms expected                                |
| medium | ~250k        | typical product mono-repo                                       |
| large  | ~700k–1M     | the regime where users hit latency walls (2M+ is rare in practice) |

## Reproducing

```bash
# Fetch corpora (one-time, ~few GB on first run)
make -C benchmarks graph-latency-fetch

# Single cell
GRAPH_LATENCY_VERSION=v1 make -C benchmarks graph-latency-run \
  GL_CORPUS=python-medium GL_TOOL=graph.search GL_PHASE=query GL_SEED=1

# Full sweep
GRAPH_LATENCY_VERSION=v1 make -C benchmarks graph-latency-sweep

# Aggregate
GRAPH_LATENCY_VERSION=v1 make -C benchmarks graph-latency-aggregate
```

## Outputs

```text
benchmarks/graph-latency/<version>/runs/<corpus>/<tool>/<phase>/<seed>.json
```

Each record (`<seed>.json`) contains `wall_ms`, `rss_peak_mb`, `result_size_bytes`, `result_count`, plus host metadata and `orbit_sha` / `corpus_sha` pins. Per-cell record schema is documented in `vN/METHOD.md`.

## Rounds

| Version | Scope | Status | Report |
|---|---|---|---|
| [v1](./v1/) | First baseline; Python + Java + TypeScript × 3 tiers × 9 tools | LIVING | [RESULTS.md](./v1/RESULTS.md) |

## Conventions

Layout, versioning, freezing rules, and the perf-RESULTS.md schema: [`../CONVENTIONS.md`](../CONVENTIONS.md).

## Directory layout

```text
benchmarks/graph-latency/
├── README.md           # this file (shared across versions)
├── scripts/            # SHARED harness
│   ├── fetch.sh        # populate ~/.cache/orbit-bench/<corpus>
│   ├── run.py          # one cell
│   ├── sweep.py        # full matrix
│   └── aggregate.py    # p50/p90/p99 tables
└── v1/                 # LIVING round 1
    ├── README.md       # version banner
    ├── METHOD.md       # corpus pins, host disclosure, record schema
    ├── RESULTS.md      # placeholder until first sweep lands
    ├── tasks/          # query-shape definitions
    └── runs/           # gitignored staging for in-progress sweep
```
