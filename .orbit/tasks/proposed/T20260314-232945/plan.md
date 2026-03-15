# Rejected To In-Progress Transition Implementation Plan

**Goal:** Allow Orbit tasks in `rejected` status to move directly to `in-progress`.
**Scope:** Update status transition validation, confirm runtime handling accepts the resumed path, and add regression coverage in core and CLI tests. Out of scope: new rejection commands, approval-policy redesign, or unrelated lifecycle changes.
**Assumptions:** Rejected tasks should keep their existing rejection metadata when resumed, and the new path should use the existing `orbit task update <id> --status in-progress` workflow.
**Risks:** Some lifecycle docs and tests currently encode `rejected -> backlog` as the only reconsideration path, and proposal-rejected versus review-rejected tasks may need slightly different assertions.

## Task 1: Update lifecycle validation

**Files:**
- Modify: `orbit-types/src/task.rs`
- Modify: `orbit-core/src/command/task.rs`
- Test: `orbit-core/src/lib.rs`

**Steps:**
1. Add or adjust failing tests for `rejected -> in-progress` in the domain and runtime layers.
2. Update transition validation so rejected tasks may resume directly to `in-progress`.
3. Confirm the update path preserves existing rejection metadata and does not regress existing guards.
4. Run targeted `orbit-core` tests covering transition behavior.
5. Update any lifecycle messaging that would otherwise contradict the new rule.

**Done When:**
- A rejected task can be resumed with `orbit task update <id> --status in-progress` and retains expected audit fields.

## Task 2: Add CLI regression coverage

**Files:**
- Modify: `orbit-cli/tests/task_commands.rs`

**Steps:**
1. Add CLI coverage for moving a rejected task directly to `in-progress`.
2. Cover both the successful transition and the expected on-disk/task-show behavior after resuming.
3. Keep existing rejection-flow coverage passing so the new path is additive.
4. Run targeted CLI task command tests.

**Done When:**
- CLI tests prove rejected tasks can re-enter active work without first moving through `backlog`.

## Final Verification
- `cargo test -p orbit-types`
- `cargo test -p orbit-core rejected review_transition_requires_execution_summary -- --nocapture`
- `cargo test -p orbit-cli task_reject -- --nocapture`