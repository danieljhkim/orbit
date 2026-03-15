Removed tool-side parameter aliasing from the Orbit builtin task/activity tools so their runtime behavior now matches dry-run validation and the published schema.

Summary of changes:
- changed `orbit_id_params` to expose only a single canonical required `id` parameter
- updated `orbit.task.show`, `orbit.task.update`, and `orbit.activity.show` to accept only `id` at the tool layer
- rewrote the orbit builtin tests to build requests with canonical `id` instead of alias keys
- updated the `open_pr`, `implement_change`, and `review_pr` activity instructions to map `input.task_id` into tool calls as `id: "{{input.task_id}}"`

Strategic decisions:
- kept the alias-to-canonical mapping at the activity call site instead of in `ToolSchema` or tool execution | Rationale: avoids schema/dry-run drift and keeps the tool contract simple | Trade-offs: callers must now be explicit about translating their own domain-specific input names into tool parameters

Assumptions made:
- the relevant agent-facing callers are the activity instruction YAMLs updated in this task | Impact if incorrect: any other caller still passing `task_id`/`activity_id` directly to these tools will now get a missing `id` error

Design weaknesses / risks:
- the activity asset YAMLs in this worktree are currently untracked repo files, so this task updates those asset sources in place rather than modifying tracked copies | Severity: Low | Mitigation: include those asset files when committing task-scoped changes if they are intended to ship

Deviations from original plan:
- None

Technical debt introduced:
- None

Recommended follow-ups:
- consider adding a small integration test around `tool run --dry-run` for Orbit builtin tools so schema/input regressions are caught outside module unit tests

Validation:
- `cargo test -p orbit-tools`
- `cargo build --workspace`
- `cargo run -q -p orbit-cli -- tool run orbit.task.show --dry-run --input '{"task_id":"T123"}'`\n- `cargo run -q -p orbit-cli -- tool run orbit.task.show --dry-run --input '{"id":"T123"}'`