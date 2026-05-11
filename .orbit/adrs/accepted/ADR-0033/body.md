## Context
CLI subprocess output emits structured tracing events after [T20260426-2313], but subscriber initialization happens before Orbit resolves a workspace root.

## Decision
Append process-level tracing events to `~/.orbit/state/logs/orbit.jsonl` through the default subscriber using the same `EnvFilter` as stderr and a retained non-blocking writer.

## Consequences
- Operators and dashboards can tail one machine-readable feed across workspaces.
- Cost: the v1 file is unrotated and concurrent processes can rarely interleave oversized JSONL records.
