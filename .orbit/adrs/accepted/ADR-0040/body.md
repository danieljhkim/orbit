## Context
Automatic task attribution is low-friction but can leave stale `planned_by` or `implemented_by` values when different actors start and finish work.

## Decision
Keep automatic stamping for plan writes and review/done transitions, but let task update callers explicitly set or clear `planned_by` and `implemented_by`.

## Consequences
- Agents can correct split or stale provenance without editing task files directly.
- Cost: attribution fields are editable metadata, so stronger authorship evidence still requires task history and audit rows.
