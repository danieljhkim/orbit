## Context
An earlier policy schema (v1) used a different shape for allow/deny rules. Supporting both shapes in the runtime caused interpretation drift between the loader, the merger, and the evaluator.

## Decision
Reject `schemaVersion: 1` at load time with an explicit migration message. v2 declares `denyRead`, `denyModify`, and `fsProfiles` and is the only accepted shape. Workspace policies override globals by profile name; global denies accumulate.

## Consequences
- Schema parsing has one supported branch, and profile authoring is uniformly `{ read, modify }` with global denies.
- Cost: existing v1 policy files require a manual migration; there is no automatic upgrader.
