# Execution Summary - Remove SQLite support for jobs and activities
Agent Name: Grace
Agent Model: GPT-5 Codex

## Status
success

## Orbit Task
Task ID: cd 

## 1. Summary of Changes
Removed SQLite-backed job and activity persistence support from the runtime and store wiring. Jobs and activities now always use the existing file-backed stores.

Updated config validation and default config assets so `job.persistence.type` and `activity.persistence.type` now reject `sqlite`, and rewrote the CLI/config tests to cover the new file-only contract.

Deleted the obsolete SQLite job/activity store implementations and their store-level integration tests, and trimmed the SQLite bootstrap/migration SQL down so it no longer creates or migrates job/activity tables.

Adjusted stale-run runtime tests to operate against file-backed job run artifacts instead of mutating SQLite state directly.

## 2. Strategic Decisions
- Keep SQLite for audits, tools, locks, and agent sessions only | Rationale: matches the narrowed task scope and avoids unintended persistence churn | Trade-offs: SQLite still exists in the codebase, just with a smaller responsibility boundary.
- Rework stale-run tests around file-backed run artifacts instead of preserving deprecated SQLite hooks | Rationale: validates the supported path directly | Trade-offs: tests no longer exercise low-level SQLite mutation APIs for jobs.
- Delete obsolete SQLite job/activity integration tests instead of rewriting them against dead APIs | Rationale: they were explicitly covering removed support | Trade-offs: store-level coverage for jobs/activities now lives in file-store and runtime tests.

## 3. Assumptions Made
- Audit-related persistence should remain SQLite-backed in this task | Impact if incorrect: more store/runtime code would need to move off SQLite in a follow-up.
- File-backed jobs and activities are the intended long-term supported path | Impact if incorrect: we would need to restore or redesign an alternate non-file backend.

## 4. Design Weaknesses / Risks
- Existing local SQLite databases may still contain historical `jobs` or `activities` tables that Orbit now ignores | Severity: Medium | Mitigation: document the new boundary and rely on file-backed persistence for current job/activity state.
- Removing SQLite config support changes behavior for users with custom legacy configs | Severity: Medium | Mitigation: config validation now fails fast with explicit `only supports 'file'` errors.\n\n## 5. Deviations from Original Plan\n- Did not change non-audit SQLite services like tools, locks, or agent sessions | Justification: the task was intentionally narrowed to jobs and activities only.\n\n## 6. Technical Debt Introduced\n- None significant in this pass | Recommended resolution: n/a\n\n## 7. Recommended Follow-Ups\n- Add a short migration note in user-facing docs if teams may still have legacy configs pointing jobs or activities at SQLite.\n- Consider whether unused legacy SQLite tables should be proactively pruned or explicitly warned about during startup.\n\n## 8. Overall Assessment\nThe codebase now enforces a cleaner persistence boundary: jobs and activities are file-backed only, audit-related SQLite behavior remains intact, and the affected CLI/core/store tests pass on the supported path.