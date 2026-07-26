---
summary: "Task Artifacts — Decisions"
type: design
title: "Task Artifacts — Decisions"
owner: codex
last_updated: 2026-07-26
status: Draft
feature: task-artifacts
doc_role: decisions
tags: ["task-artifacts"]
---

# Task Artifacts — Decisions

ADR log for the task-artifacts feature. Format follows [docs/design/CONVENTIONS.md §4](../CONVENTIONS.md): each entry is `Context · Decision · Consequences`, every entry names at least one Cost, and numbers are append-only.

ADR numbers are global, allocated via `orbit.adr.add` before the local heading is written. Cross-folder references use full paths. ADRs whose status is `Proposed` flip to `Accepted` when their implementing task lands; the implementing task ID is appended to the Status line.

Historical note ([ORB-10458]): the entries listed below were authored with local IDs that had no record in the ADR store. They were allocated through `orbit.adr.add`, their narratives migrated into the store verbatim, and their headings rewritten to the allocated global ID. The original local IDs survive as `legacy_ids`, so prior citations still resolve via `orbit tool run orbit.adr.show --input '{"legacy_id":"<feature>/ADR-NNN"}'`. Backfilled here: `task-artifacts/ADR-001` → ADR-0261, `task-artifacts/ADR-002` → ADR-0262, `task-artifacts/ADR-003` → ADR-0263, `task-artifacts/ADR-004` → ADR-0264, `task-artifacts/ADR-005` → ADR-0265, `task-artifacts/ADR-006` → ADR-0266, `task-artifacts/ADR-007` → ADR-0267, `task-artifacts/ADR-008` → ADR-0268, `task-artifacts/ADR-009` → ADR-0269.

---

## ADR-0261 — Authority-scoped `ORB-00000` task IDs

**Status:** Accepted · 2026-05 · Phase 1 v2 domain contracts (`c1f72a32`); Phase 2 home registry allocator (`1ae83804`); legacy gate removed (`e9582eba`) · legacy_id: `task-artifacts/ADR-001`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0261"}'`.

---

## ADR-0262 — Envelope YAML plus Markdown sidecars for prose

**Status:** Accepted · 2026-05 · Phase 3 v2 bundle primitives (`c14fa640`); Phase 4 document update hardening (`06847332`) · legacy_id: `task-artifacts/ADR-002`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0262"}'`.

---

## ADR-0263 — Status-neutral task directories

**Status:** Accepted · 2026-05 · Phase 3 v2 runtime backend (`3be9bd5f`, `c14fa640`); Phase 6 legacy gate removed (`e9582eba`) · legacy_id: `task-artifacts/ADR-003`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0263"}'`.

---

## ADR-0264 — Append-heavy task data leaves `task.yaml`

**Status:** Accepted · 2026-05 · Phase 3 v2 bundle primitives (`c14fa640`); Phase 4 hardening of append/tail-repair (`06847332`) · legacy_id: `task-artifacts/ADR-004`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0264"}'`.

---

## ADR-0265 — Typed relations over scattered link fields

**Status:** Accepted · 2026-05 · Phase 6 relations and job-run wiring (working tree) · legacy_id: `task-artifacts/ADR-005`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0265"}'`.

---

## ADR-0266 — Artifact manifest with binary-capable files

**Status:** Accepted · 2026-05 · Phase 6 public artifact DTO surgery (working tree) · legacy_id: `task-artifacts/ADR-006`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0266"}'`.

---

## ADR-0267 — Home task store with workspace symlink projection

**Status:** Accepted · 2026-05 · Phase 2 home registry foundation (`1ae83804`); Phase 3 v2 runtime backend and symlink projection (`3be9bd5f`, `c14fa640`) · legacy_id: `task-artifacts/ADR-007`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0267"}'`.

---

## ADR-0268 — Forward-only YAML migration framework in `orbit-common`

**Status:** Accepted · 2026-05 · Forward-only YAML migration framework (`01928e76`) · legacy_id: `task-artifacts/ADR-008`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0268"}'`.

---

## ADR-0269 — Cross-artifact provenance uses `produces` and `resolves`

**Status:** Accepted · 2026-05 · ORB-00093 · legacy_id: `task-artifacts/ADR-009`

Narrative lives in the ADR store — retrieve it with `orbit tool run orbit.adr.show --input '{"id":"ADR-0269"}'`.

---

## ADR-0182 — Review-thread hook active task binding

**Status:** Accepted · 2026-05 · [ORB-00273]

**Context.** Review-thread reminders need a cheap way to know which task owns the current agent turn. Inferring from cwd or scanning task files would make every PreToolUse call depend on filesystem heuristics, while the engine already knows the executing task when it seeds `ORBIT_TASK_ID`.

**Decision.** The hook treats `ORBIT_ACTIVE_TASK_ID` as the explicit active-task binding, with `ORBIT_TASK_ID` as a compatibility fallback for existing execution paths. Orbit execution code seeds both values when the activity input contains a task id, and hook state is still scoped by the existing session id plus parent-pid state-file key.

**Consequences.**
- Review-thread surfacing remains a local task-store read and does not perform network I/O or cwd inference.
- Existing `ORBIT_TASK_ID`-spawned executions keep working while newer shims can depend on the clearer `ORBIT_ACTIVE_TASK_ID` name.
- Cost: Orbit now has two task-id environment names during a compatibility window, so documentation and tests must keep their precedence explicit.

---

## Task References

- ORB-00093
- ORB-00273

Resolve any task above with `orbit task show <ID>` or `git log --grep=<ID>`.
