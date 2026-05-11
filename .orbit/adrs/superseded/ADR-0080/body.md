## Context
`orbit.graph.search` added an exact `task_id` filter in [T20260426-0220], but its input validator hardcoded `T\d{8}-\d{4}`. The task store now creates unpadded daily suffixes such as `T20260428-1`, while historical graph/task references also include amended numeric suffixes such as `T20260412-0645-2`. The graph attribution default still only matched the older four-digit base suffix, so a selector-first task lookup could fail before search, or miss current task IDs after a rebuild.

## Decision
Treat the bare Orbit task-ID body accepted by graph attribution/search as `T\d{8}-\d+(?:-\d+)*`. Keep the configurable `TaskIdPattern` mechanism from ADR-020; this change only updates Orbit's default pattern and the agent-facing `orbit.graph.search` input validator.

## Consequences
- Current task-store IDs, historical four-digit IDs, and amended numeric IDs all share one graph default.
- A workspace with a manifest written under the older default will see the existing manifest-pattern mismatch path and get a full-history backfill on the next graph build.
- Cost: the default is intentionally more permissive about leading zeros and amendment depth so existing historical IDs stay queryable; task creation remains governed by the task store.

---
