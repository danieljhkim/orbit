# graph/v4 — LIVING

**Status:** LIVING — round 4 in design. Fixtures and run records under this directory are mutable until freeze.

v4 is a **diagnostic** round, not a keep/cull round. v3 settled retention of the agent-facing `orbit_graph_*` MCP surface; v4 maps where the surface helps, where it hurts, and how it fails — so future tool-shaping work has measured targets.

- Method + pre-registered report shape: [`METHOD.md`](./METHOD.md)
- Fixtures: [`./tasks/`](./tasks/) (12 NEW fixtures; no v1/v2/v3 carries)
- Synthetic-fixture code island: [`./_fixture_code/`](./_fixture_code/) (re-included in `.orbitignore` via narrow negation; see METHOD)
- Staging for in-progress sweep data: [`./runs/`](./runs/) (gitignored until freeze)
- Shared scripts: [`../scripts/`](../scripts/)
- Shared harness overview: [`../README.md`](../README.md)
- Prior frozen rounds: [`../v3/`](../v3/), [`../v2/`](../v2/), [`../v1/`](../v1/)
- Closing entry on the v1–v3 evidence series: [`../../../docs/design/knowledge-graph/5_null_result.md`](../../../docs/design/knowledge-graph/5_null_result.md)

Running a single cell against v4:

```bash
GRAPH_VERSION=v4 python3 benchmarks/graph/scripts/run.py \
  --provider codex --arm hybrid --task callers-2hop-graphbenchpolicy --seed 1
```

See [`../../CONVENTIONS.md`](../../CONVENTIONS.md) for the freeze procedure when round 4 concludes.
