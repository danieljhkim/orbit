# Knowledge Graph Token-Usage Benchmarks

Measures how much token budget an agent spends to solve the same task when it has access to different navigation toolsets. The goal is a repeatable, comparable number — not a one-off demo.

See [docs/design/knowledge-graph/](../../docs/design/knowledge-graph/) for what the graph is.

## Arms

Each run locks the agent to one of three tool allowlists:

| Arm          | Allowed navigation tools                                    | Graph pre-built? |
|--------------|-------------------------------------------------------------|------------------|
| `no-graph`   | `Read`, `Grep`, `Glob`, `Bash(rg/ls/git log/git grep)`      | no               |
| `graph-only` | `orbit.graph.{overview,search,show,pack,callers,implementors}` | yes           |
| `hybrid`     | union of both                                               | yes              |

Allowlists live under [`arms/`](./arms/) as Claude Code `settings.json` fragments. Non-navigation tools (`Edit`, `Write`, `Bash(cargo ...)`) are permitted in all arms so the agent can actually complete edit tasks.

## Task suite

Three classes × three difficulty tiers, ~2 fixtures each (~18 total):

- **locate** — "Where is X defined, what kind is it, what implements/uses it?" The graph's sweet spot.
- **trace**  — "Walk callers of X to depth 2 and summarize what each caller does." Tests deeper navigation.
- **edit**   — A bounded code change with a compile-and-test oracle. Tests whether graph lookups survive the handoff to actual editing.

Each fixture is a YAML under [`tasks/`](./tasks/) pinning:

- `commit_sha` — exact repo state to run against
- `prompt` — verbatim user message
- `oracle` — one of:
  - `grep:` assertion (substring must / must not appear in final diff or named file)
  - `cmd:` shell command that must exit 0 (e.g. `cargo test -p orbit-knowledge -- test_foo`)
  - `judge:` LLM-judge rubric (used as fallback for open-ended trace/summary tasks)

See [`tasks/_schema.yaml`](./tasks/_schema.yaml) for the full schema.

## Controls

- Single model + version across a sweep; `temperature=0`; fixed system-prompt scaffold in [`prompts/system.md`](./prompts/system.md).
- Fresh sandbox per run: clone repo to `/tmp/orbit-bench-<uuid>`, checkout `commit_sha`, rebuild the graph for arms that need it, no prior prompt cache.
- `N=5` runs per cell → `3 arms × 18 tasks × 5 = 270` runs per sweep.

## Metrics (per run)

Recorded as `runs/<arm>/<task_id>/<seed>.json`:

- `input_tokens`, `cache_read_tokens`, `cache_creation_tokens`, `output_tokens`, `total_cost_usd`
- `wall_seconds`, `turns`, `tool_calls` (histogram by tool name)
- `verdict` ∈ `{pass, fail, error}` from the oracle, plus `judge_rationale` when the judge fires
- `transcript_path`, `final_diff_path` — full artifacts, gitignored

## Aggregation

`scripts/aggregate.py` reads `runs/` and emits a table per sweep:

- median and p90 of `input_tokens + output_tokens` per (arm × task_class)
- pass rate per (arm × task_class)
- **tokens-per-success** = total tokens across runs ÷ number of passes (the headline number)

## Directory layout

```
benchmarks/graph/
├── README.md          ← this file
├── arms/              ← per-arm Claude Code settings fragments
├── prompts/           ← shared system prompt + per-arm preambles
├── tasks/             ← fixture YAMLs + schema
├── scripts/           ← run driver, judge, aggregator (not yet written)
└── runs/              ← gitignored result artifacts
```

## Status

Design committed; scripts not yet implemented. Tracked as an Orbit task before any runs are executed.
