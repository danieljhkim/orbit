## Context
A scheduled ship routine must dispatch only its source workspace, resolve that workspace ship mode and base branch, and keep the parent run active until normal backlog shipment finishes. The alternatives were special-casing routine dispatch, spawning the legacy multi-workspace CLI sweep, or composing the existing job catalog.

## Decision
Seed a workspace-local ship-sweep routine targeting a shipped wrapper job. The wrapper deterministically resolves ship input for its active runtime, invokes `task_auto_pipeline` with no explicit task IDs, waits for it, and guards child success; it does not consult `workflow.auto_ship` or the cross-workspace sweep path.

## Consequences
- Backlog discovery, readiness, locking, bundling, crew selection, and gates remain owned by `task_auto_pipeline`.
- `overlap: forbid` covers the child shipment because the wrapper does not finish before the child.
- The legacy global ship-sweep remains compatible during burn-in but is not used by routines.
- Cost: the catalog gains a small wrapper job and deterministic resolver activity whose input contract must stay aligned with the canonical ship workflow.