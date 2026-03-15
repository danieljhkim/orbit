# Workspace Path Template Source Clarification

**Goal:** Define and document the canonical source for `{{workspace_path}}` in activity template rendering.
**Scope:** activity execution model, template context, and user-facing docs/tests.
**Assumptions:** Current behavior is ambiguous enough that it should be resolved intentionally rather than by convention.
**Risks:** Leaving this undefined can cause CLI/API activities to execute in the wrong directory or render incorrect paths.

## Task 1: Choose canonical source

**Files:**
- Modify: `orbit-types/src/activity.rs` or related execution-context types
- Modify: `orbit-core/src/command/job.rs`
- Modify: documentation/tests as needed

**Steps:**
1. Decide whether `workspace_path` belongs on Activity, JobStep, Job, or should resolve from repo/runtime context.
2. Update code and docs to make that source explicit.
3. Add regression coverage for the chosen behavior.

**Done When:**
- `{{workspace_path}}` has a clearly defined and tested source with no agent guesswork required.

## Final Verification
- `cargo test --workspace`