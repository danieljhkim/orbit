# graph/v4 — FROZEN

**Status:** FROZEN snapshot of round 4, frozen 2026-04-25. Do not modify files in this directory except for explicit benchmark-result errata.

v4 is a **diagnostic** round, not a keep/cull round. v3 settled retention of the agent-facing `orbit_graph_*` MCP surface; v4 maps where the surface helps, where it hurts, and how it fails — so future tool-shaping work has measured targets.

- Method + pre-registered report shape: [`METHOD.md`](./METHOD.md)
- Report: [`RESULTS.md`](./RESULTS.md)
- Fixtures: [`./tasks/`](./tasks/) (12 NEW fixtures; no v1/v2/v3 carries)
- Synthetic-fixture code island: [`./_fixture_code/`](./_fixture_code/) (re-included in `.orbitignore` via narrow negation; see METHOD)
- Frozen run data: [`./runs/`](./runs/)
- Shared scripts: [`../scripts/`](../scripts/)
- Shared harness overview: [`../README.md`](../README.md)
- Prior frozen rounds: [`../v3/`](../v3/), [`../v2/`](../v2/), [`../v1/`](../v1/)
- Follow-up validation round: [`../v5/`](../v5/)
- Closing entry on the v1–v3 evidence series: [`../../../docs/design/knowledge-graph/5_null_result.md`](../../../docs/design/knowledge-graph/5_null_result.md)

Re-running a single cell against frozen v4:

```bash
GRAPH_VERSION=v4 python3 benchmarks/graph/scripts/run.py \
  --provider codex --arm hybrid --task callers-2hop-graphbenchpolicy --seed 1
```

See [`../../CONVENTIONS.md`](../../CONVENTIONS.md) for version-freeze rules.
