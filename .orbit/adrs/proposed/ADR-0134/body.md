## Context
The `T<YYYYMMDD>-<N>` task ID format is a local interface, not a cross-machine reference. It appears in commit messages (`[T20260421-0528]`), local audit events, and operator scripts that grep `T\d{8}-\d+`. Knowledge-graph node attribution was removed in [T20260506-11], and cross-engineer task references now go through `external_refs`. The current allocator at [layout.rs:102-132](../../../crates/orbit-store/src/file/task_store/layout.rs) scans the local file store and increments — it has no view of other operators' allocations.

## Decision
ID allocation continues to produce `T<YYYYMMDD>-<N>`; the format is preserved. The allocator gains a "view" abstraction that, when sync is enabled, scans the fetched registry's state directories *plus* any local unpushed tasks before computing the next counter. On push rejection caused by ID collision, the operation is retried via the standard replay path: re-fetch, re-allocate (now seeing the conflicting peer's task), rewrite the bundle locally with the new ID, and retry push. The retry window is safe because allocation happens before any commit message, audit event, or agent dispatch references the ID.

## Consequences
- All existing consumers of `T<YYYYMMDD>-<N>` continue to work.
- Allocator becomes view-aware but does not change format or storage.
- Cost: ID allocation requires the registry fetch — `task add` becomes online-only when sync is enabled. This is the largest behavioral change exposed to users.

---
