# Knowledge Graph Benchmarks

kind: agent

Measures how much navigation budget an agent spends on the same task under three tool surfaces: `no-graph` (shell only), `graph-only` (Orbit graph MCP tools only), or `hybrid` (both).

See [docs/design/knowledge-graph/](../../docs/design/knowledge-graph/) for the graph itself.

**Series closed.** Read [`RESULTS.md`](./RESULTS.md) for the cross-round synthesis and findings.

## Rounds

| Version | Scope | Report |
|---|---|---|
| [v1](./v1/) | Initial baseline | [RESULTS.md](./v1/RESULTS.md) |
| [v2](./v2/) | Extended fixtures | [RESULTS.md](./v2/RESULTS.md) |
| [v3](./v3/) | Calibrated cost; published null result | [RESULTS.md](./v3/RESULTS.md) |
| [v4](./v4/) | Diagnostic round, 192 planned cells plus Codex post-fix graph-only rerun | [RESULTS.md](./v4/RESULTS.md) |
| [v5](./v5/) | Feature validation (`source_regex`), 9 cells | [RESULTS.md](./v5/RESULTS.md) |

All rounds frozen.

## Reproducing

```bash
# Single cell
GRAPH_VERSION=v4 benchmarks/graph/scripts/run.sh graph-only reverse-export-orbit-error 1 --provider codex

# Sweep
GRAPH_VERSION=v4 python3 benchmarks/graph/scripts/sweep.py --provider codex --arms graph-only --n 3

# Aggregate
GRAPH_VERSION=v4 python3 benchmarks/graph/scripts/aggregate.py \
  --runs benchmarks/graph/v4/runs --tasks benchmarks/graph/v4/tasks
```

## Outputs

```text
benchmarks/graph/<version>/runs/<provider>/<arm>/<task_id>/<seed>.json
benchmarks/graph/<version>/runs/_sweeps/<provider>/<sweep_id>/order.json
```

Records (`<seed>.json`) include verdict, token counts, wall time, and tool-call histogram. v1-v4 retain full transcripts (`<seed>.transcript.json`); v5 is records-only.

## Conventions

Version freezing rules and round structure: [`../CONVENTIONS.md`](../CONVENTIONS.md).

## Directory Layout

```text
benchmarks/graph/
├── README.md        # this file (shared across versions)
├── v1/              # FROZEN round 1
│   ├── README.md    # version-specific banner
│   ├── METHOD.md
│   ├── RESULTS.md
│   ├── mcp.json
│   ├── scripts/     # harness as it ran
│   ├── tasks/       # fixtures as they were graded
│   └── runs/        # per-cell records
├── v2/              # FROZEN round 2
│   └── ...
├── v3/              # FROZEN round 3
│   └── ...
├── v4/              # FROZEN round 4
│   └── ...
└── v5/              # FROZEN round 5
    └── ...
```
