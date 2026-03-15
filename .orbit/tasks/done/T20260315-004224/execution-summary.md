# Execution Summary - Clarify source of {{workspace_path}} for activity template rendering
Agent Name: Claude
Agent Model: claude-sonnet-4-6

## Status
success

## Orbit Task
Task ID: T20260315-004224

## 1. Summary of Changes
Added `workspace_path: Option<String>` as an explicit top-level field on `Activity`, threaded through all layers (orbit-types → orbit-store → orbit-core → orbit-cli). Updated `execution_template_context` in job.rs to use `activity.workspace_path` directly instead of silently defaulting to `data_root`. `{{workspace_path}}` now returns a clear error if the activity has no workspace_path set, rather than silently resolving to the Orbit data root (.orbit directory).

Files changed:
- `orbit-types/src/activity.rs` — added `workspace_path: Option<String>`
- `orbit-store/src/backend/contracts.rs` — added to `ActivityCreateParams` and `ActivityUpdateParams`
- `orbit-store/src/file/activity_store.rs` — added to `FileWorkInsert`, `ActivitySpecDocument`, `update_activity`, `doc_to_work`
- `orbit-store/src/backend/file_backends.rs` — threaded through bridge
- `orbit-core/src/command/activity.rs` — added to `ActivityAddParams`, `ActivityUpdateParams`, `ActivityFileSpec`, seeding path
- `orbit-core/src/command/job.rs` — `execution_template_context` now uses `activity.workspace_path`
- `orbit-cli/src/command/activity.rs` — `--workspace-path` and `--clear-workspace-path` flags, `activity_to_json` output
- `orbit-cli/tests/activity_commands.rs` — new test `activity_workspace_path_is_stored_and_clearable`
- `orbit-core/tests/job_runtime_behavior.rs` — updated `cli_command_activity_executes_without_agent_cli_and_captures_output_file` to set `workspace_path` explicitly
- `orbit-types/src/lib.rs`, `orbit-core/src/lib.rs` — updated test struct initializers

## 2. Strategic Decisions
- Add as top-level field (not inside spec_config) | Rationale: workspace_path is a cross-cutting meta-field applicable to all spec_types, not a type-specific config detail | Trade-offs: slightly more surface area in the Activity struct vs. keeping spec_config as the single blob
- Default to None / return error when unset | Rationale: makes missing configuration explicit rather than silently misdirecting execution to .orbit directory | Trade-offs: activities that previously relied on the implicit data_root default (if any existed in the wild) would break — but this was undocumented behavior
- Use `skip_serializing_if = "Option::is_none"` on YAML | Rationale: keeps existing activity YAML files clean — no `workspace_path: null` noise for the common case

## 3. Assumptions Made
- No production activity YAMLs in `.orbit/activities/active/` rely on `{{workspace_path}}` resolving to `data_root` implicitly | Impact if incorrect: those activities would need `workspace_path:` added to their YAML
- The test `cli_command_activity_executes_without_agent_cli_and_captures_output_file` was previously working by coincidence (data_root == tempdir) | Impact if incorrect: N/A — fixed regardless

## 4. Design Weaknesses / Risks
- `{{workspace_path}}` error message says "workspace_path is unavailable in this context" — slightly cryptic for users who forgot to set it | Severity: Low | Mitigation: could be improved to "activity has no workspace_path configured; set it with orbit activity update --workspace-path"
- `workspace_path` stored as a plain string, not validated as a path | Severity: Low | Mitigation: runtime errors from the OS will surface invalid paths immediately

## 5. Deviations from Original Plan
- Plan said "decide whether workspace_path belongs on Activity, JobStep, Job, or should resolve from repo/runtime context" — chose Activity as the owner | Justification: most explicit, consistent with Task.workspace_path pattern, no implicit global state

## 6. Technical Debt Introduced
- None

## 7. Recommended Follow-Ups
- Update the error message in template.rs for missing `workspace_path` to be more actionable
- Consider adding `workspace_path` display to `orbit activity show` (text mode currently omits it)

## 8. Overall Assessment
Clean, minimal fix. The ambiguity is fully resolved — workspace_path is explicit, optional, and fails loudly when missing. The existing runtime test now serves as a regression test for the explicit behavior.