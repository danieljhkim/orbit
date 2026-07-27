## Context

Task bundles need to be close to the workspace so agents can inspect and update them with project context, but keeping the canonical copy inside every checkout makes gitignored task data fragile. `~/.orbit` already needs to allocate IDs and remember workspace bindings, so it can own canonical local task storage while the checkout exposes a projection.

## Decision

Treat `~/.orbit/tasks/workspaces/<workspace-id>/<task-id>/` as the canonical local bundle and `.orbit/tasks/<task-id>` as a symlink projection. Store `workspace_id` in `.orbit/config.yaml` and mandatory allocator, workspace-binding, local execution overlay, status, relation, tag, and lock/index metadata under `~/.orbit/tasks/index.sqlite`.

## Consequences


- Task artifacts remain addressable next to the code without making the checkout the canonical store.
- Allocation and workspace resolution are durable without making every content write a dual-write operation.
- Deleting `.orbit/tasks/` only removes projection links; Orbit can rebuild them from `.orbit/config.yaml` and `index.sqlite`.
- Sync and hosted modes can replace or augment allocation without changing the workspace bundle shape.
- Cost: `.orbit/config.yaml` becomes load-bearing for binding. If it is lost, Orbit must rebind by path/repo fingerprints or prompt the user; symlink-restricted filesystems need a degraded projection fallback.

## Provenance

Migrated verbatim from the local heading `task-artifacts/ADR-007` in `docs/design/task-artifacts/4_decisions.md` by [ORB-10458]. Original status line: Accepted · 2026-05 · Phase 2 home registry foundation (`1ae83804`); Phase 3 v2 runtime backend and symlink projection (`3be9bd5f`, `c14fa640`)