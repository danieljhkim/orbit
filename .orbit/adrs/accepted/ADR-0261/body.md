## Context

The current `T<YYYYMMDD>-<N>` format is allocated by scanning one workspace's task directories. That shape is useful as a local search key but fails the moment tasks need to be referenced across local workspaces, through an explicit registry, hosted Team, or durable design docs without explaining which machine allocated them.

## Decision

Adopt `ORB-00000` as the canonical v2 task ID format: `ORB-` plus a five-digit decimal suffix allocated by an explicit authority. The ID is unique inside that authority, not across unrelated local registries. V2 task bundles do not preserve old `T...` identifiers as aliases.

## Consequences

- Task IDs become meaningful inside the scope of the configured allocator instead of implicitly workspace-local.
- Local-only Orbit uses one allocator across all local workspaces, so one machine does not mint the same ID for two repositories.
- Two unrelated local registries may both allocate the same bare ID; cross-registry references must carry registry, workspace, hosted tenant, or external-reference context.
- Implementations must stop validating only `T<YYYYMMDD>-<N>` and must add numeric `ORB-\d{5}` validation.
- Existing local tasks need a cutover command, but the result is a clean v2 task store rather than a dual-ID store.
- Cost: task creation now depends on an allocator outside the task directory scan. Sync and hosted modes need shared allocation before a task can be published.

## Provenance

Migrated verbatim from the local heading `task-artifacts/ADR-001` in `docs/design/task-artifacts/4_decisions.md` by [ORB-10458]. Original status line: Accepted · 2026-05 · Phase 1 v2 domain contracts (`c1f72a32`); Phase 2 home registry allocator (`1ae83804`); legacy gate removed (`e9582eba`).