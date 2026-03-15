Added a new builtin `orbit` tool namespace in `orbit-tools/src/builtin/orbit/` and wired it into builtin registration so agents can read and update Orbit data through tools instead of shelling out ad hoc.

Summary of changes:
- added `orbit.task.show`, `orbit.task.list`, `orbit.task.update`, and `orbit.activity.show` builtin tools
- implemented shared Orbit CLI helpers for request construction, JSON parsing, and consistent execution error handling
- made the tools honor `ToolContext.cwd` when provided, while still invoking the `orbit` binary via `run_process`
- added focused tests for registration, missing-ID validation, status filtering, update request construction, and `task_id` / `activity_id` aliases

Strategic decisions:
- `orbit.task.update` always uses an update-then-show flow instead of relying on `orbit task update --json`, because the current CLI help does not expose `--json` there
- `orbit.task.show` and `orbit.activity.show` accept `task_id` / `activity_id` aliases in addition to `id` so the shipped activity YAML can use domain-specific input names immediately

Follow-up work:
- created `T20260315-032558` to track the remaining schema/dry-run limitation around required parameter aliases

Validation:
- `cargo test -p orbit-tools orbit_tools_are_registered -- --nocapture`
- `cargo test -p orbit-tools task_update_builds_update_and_show_requests -- --nocapture`
- `cargo test -p orbit-tools`
- `cargo build --workspace`
- `cargo run -q -p orbit-cli -- tool list | rg 'orbit\\.(task\\.(show|list|update)|activity\\.show)'