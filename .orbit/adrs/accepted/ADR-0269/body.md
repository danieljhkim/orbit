## Context

Task records already carry a typed `relations` array, but every relation type was task-only. Non-task artifact provenance was split across one-way back-pointers: `FrictionRecord.during_task`, ADR `related_tasks`, and learning evidence. That fragmentation made "what did task T touch?" artifact-specific, and made friction closure manual even when a task explicitly fixed the friction.

## Decision

Add two cross-artifact relation types to the task envelope: `produces` for artifacts created during execution and `resolves` for artifacts closed or superseded by the task. These two relation types accept task, friction, learning, and ADR ID shapes (`ORB-`, `FYYYY-MM-NNN`, `L-NNNN`, `ADR-NNNN+`). Existing relation types remain task-only. Friction auto-close is the only v1 side effect: when a task moves from Review to Done, `resolves -> F...` transitions the friction to `resolved` and records `resolved_by_task`.

## Consequences


- Agents get one typed provenance surface for task-created and task-closed artifacts without migrating historical ADR, learning, or friction fields.
- Frictions can close automatically as part of approval while dangling friction references stay audit-visible instead of blocking task completion.
- Learning citation work can depend on explicit `produces -> L...` edges rather than regex-scanning task prose.
- Cost: the relation validator now has a cross-artifact branch and a task-only branch, so tests must protect legacy relation strictness.
- Cost: `task_bundle_relations.target_task_id` now stores non-task IDs for `produces` / `resolves`; task-target inverse lookups must continue validating callers that expect task IDs.

## Provenance

Migrated verbatim from the local heading `task-artifacts/ADR-009` in `docs/design/task-artifacts/4_decisions.md` by [ORB-10458]. Original status line: Accepted · 2026-05 · ORB-00093