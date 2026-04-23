# Knowledge Graph Benchmarks

Measures how much navigation budget an agent spends solving the same task under different tool surfaces. The benchmark is meant to compare repeatable experiment cells, not produce one-off anecdotes.

See [docs/design/knowledge-graph/](../../docs/design/knowledge-graph/) for the graph itself.

## Providers

The harness supports two CLI providers:

| Provider | Navigation surface in this benchmark |
|---|---|
| `claude` | Native Claude Code tools plus Orbit MCP graph tools |
| `codex` | Native Codex `exec_command`; graph access is Phase 1 shell-driven via `orbit tool run orbit.graph.*` |

Codex does not currently use direct MCP graph calls in this harness. In Phase 1, its graph path is provider-native shell execution from the repo root.

## Arms

Each run locks the child session to one of three navigation modes:

| Arm | Claude behavior | Codex behavior |
|---|---|---|
| `no-graph` | `Read`, `Grep`, `Glob`; graph tools denied | shell-only filesystem navigation (`rg`, `ls`, `find`, focused reads); `orbit.graph.*` forbidden by prompt and classified as an error if used |
| `graph-only` | Orbit graph MCP tools only | shell-only, but the intended surface is `orbit tool run orbit.graph.*`; zero graph calls with zero denials is classified as an error |
| `hybrid` | filesystem tools plus graph MCP tools | shell-only with both graph commands and filesystem commands allowed |

Claude keeps native allowlists. Codex is provider-native in Phase 1, so the arm boundary is enforced through prompt steering plus post-run classification of observed commands.

## Running

Run a single cell:

```bash
benchmarks/graph/scripts/run.sh graph-only locate-agentruntime 1 --provider claude
benchmarks/graph/scripts/run.sh graph-only locate-agentruntime 1 --provider codex
make -C benchmarks graph-run GRAPH_PROVIDER=codex GRAPH_ARM=graph-only GRAPH_TASK=locate-agentruntime
```

Run a sweep:

```bash
python3 benchmarks/graph/scripts/sweep.py --provider claude --n 5
python3 benchmarks/graph/scripts/sweep.py --provider codex --n 5
make -C benchmarks graph-sweep GRAPH_PROVIDER=codex GRAPH_N=1 GRAPH_TASKS=locate-agentruntime
```

Useful flags:

- `--provider {claude,codex}` selects the child CLI.
- `--no-probe` skips the graph pre-flight check for graph-enabled arms.
- `--budget` sets the Claude spend hint for a single run. Codex records cost as `0.0` in Phase 1 because its JSON event stream does not expose spend.

## Tasks

Fixtures live under [`tasks/`](./tasks/) and pin:

- `commit_sha`: exact repo state to run against
- `prompt`: verbatim user message
- `oracle`: grading rule for the final assistant message

See [`tasks/_schema.yaml`](./tasks/_schema.yaml) for the schema.

## Outputs

Per-run records live under:

```text
benchmarks/graph/runs/<provider>/<arm>/<task_id>/<seed>.json
benchmarks/graph/runs/<provider>/<arm>/<task_id>/<seed>.transcript.json
```

Each run record includes:

- `provider`, `requested_model`, `arm`, `task_id`, `seed`
- token counts, wall time, total cost, tool-call histogram
- normalized `model_usage`
- `verdict` in `{pass, fail, error}` plus a diagnostic
- transcript artifact paths

Sweep metadata lives under:

```text
benchmarks/graph/runs/_sweeps/<provider>/<sweep_id>/
```

## Aggregation

`python3 benchmarks/graph/scripts/aggregate.py` reads the run tree and prints:

- a primary table grouped by `(provider, arm, task_class)`
- a secondary table grouped by `(provider, model, arm, task_class)`
- an error table for excluded runs

## Directory Layout

```text
benchmarks/graph/
├── README.md
├── tasks/
├── scripts/
└── runs/
```
