## Context
ADR-0049 correctly separated friction reports from planned task work, but coupled that semantic decision to Markdown files under `.orbit/frictions/`. File backing was reasonable while records were low-volume, Git-visible, and directly inspectable. Frictions are now hub-only coordination state, authors and operators mutate them through Orbit surfaces, and every filtered list or stats request parses and materializes the complete retained file corpus. The real alternatives are to retain per-record Markdown for direct inspection or preserve the artifact semantics while moving live persistence to indexed SQLite.

## Decision
Keep friction as a first-class operational artifact outside the task lifecycle, with the existing `orbit.friction.*`, CLI, HTTP, dashboard, Bridge, status, tag, resolution, and task-relation semantics. Persist live friction records in the global Orbit SQLite store under composite identity `(workspace_id, friction_id)`, push filtering, ordering, pagination, and aggregation into SQL, and allocate workspace-local monthly IDs transactionally. Small tag-taxonomy configuration may remain file-backed. Direct inspection and portability are provided through supported show/list/export surfaces rather than live Markdown records.

## Consequences
- Task backlogs remain free of self-report signal; this decision does not restore a `friction` task status.
- Fixed-size friction pages decode only their result rows, so scan memory no longer grows with retained friction history.
- Identical friction IDs may safely coexist in different workspaces and every read/write remains explicitly workspace-scoped.
- Legacy Markdown records remain migration evidence for one release but cease to be a live source after a workspace import commits.
- Cost: Raw per-record file inspection and Git diffs are no longer the persistence interface; operators depend on SQLite backup/integrity tooling and Orbit export surfaces for recovery and review.