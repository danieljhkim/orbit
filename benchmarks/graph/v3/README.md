# graph/v3 — FROZEN

**Status:** FROZEN snapshot of round 3, frozen 2026-04-23. Do not modify files in this directory.

- Method + pre-registered disposition: [`METHOD.md`](./METHOD.md)
- Report: [`RESULTS.md`](./RESULTS.md)
- Fixtures: [`./tasks/`](./tasks/)
- Frozen run data: [`./runs/`](./runs/)
- Shared scripts: [`../scripts/`](../scripts/)
- Shared harness overview: [`../README.md`](../README.md)
- Prior frozen report: [`../v2/RESULTS.md`](../v2/RESULTS.md)

Re-running a single cell against frozen v3:

```bash
GRAPH_VERSION=v3 python3 benchmarks/graph/scripts/run.py \
  --provider claude --arm hybrid --task locate-agentruntime --seed 1
```

Or via the Makefile (defaults to `GRAPH_VERSION=v3`):

```bash
make -C benchmarks graph-run GRAPH_PROVIDER=claude GRAPH_ARM=hybrid GRAPH_TASK=locate-agentruntime
```

See [`../../CONVENTIONS.md`](../../CONVENTIONS.md) for version-freeze rules.
