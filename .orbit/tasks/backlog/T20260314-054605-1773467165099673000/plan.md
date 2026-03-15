# Task Lifecycle Ergonomics Plan

**Goal:** Make the common task execution path simpler while preserving Orbit's explicit lifecycle and audit trail.
**Scope:** Task CLI commands, runtime lifecycle helpers, workflow docs, and task command tests.
**Assumptions:** The most painful path today is proposal approval followed immediately by assignment/start work.
**Risks:** Poorly designed shortcuts could hide lifecycle intent or create overlapping ways to do the same thing.

## Task 1: Design the streamlined lifecycle command

**Files:**
- Modify: `orbit-cli/src/command/task.rs`
- Modify: `orbit-core/src/command/task.rs`
- Modify: `orbit-types/src/task.rs` only if lifecycle helpers need extension

**Steps:**
1. Choose the ergonomic surface for the common path (for example `task start`, or `task approve --assigned-to ... --start`).
2. Define exactly which transitions it covers and which approval/audit fields it writes.
3. Add failing tests for the intended flow before implementation.

**Done When:**
- Orbit has one clear shortcut for the common approval-to-execution path.
- The contract is explicit about status transitions and audit metadata.

## Task 2: Implement runtime/CLI support without weakening safeguards

**Files:**
- Modify: `orbit-core/src/command/task.rs`
- Modify: `orbit-cli/src/command/task.rs`
- Modify: task event/audit handling if needed for the new flow

**Steps:**
1. Implement the new streamlined command path.
2. Preserve required lifecycle checks such as valid status transitions and execution-summary rules.
3. Ensure the resulting task metadata remains auditable and human-readable.

**Done When:**
- The common path no longer requires multiple manual commands.
- Orbit still enforces the same lifecycle safeguards.

## Task 3: Update tests and workflow guidance

**Files:**
- Modify: `orbit-cli/tests/task_commands.rs`
- Modify: `orbit-core/assets/skills/orbit-approve-task/SKILL.md`
- Modify: `orbit-core/assets/skills/orbit-manage-tasks/SKILL.md`
- Modify: mirrored `.orbit` skill copies if they remain part of the shipped workflow surface

**Steps:**
1. Add CLI coverage for the new flow and failure cases.
2. Update guidance so agents use the simplified path instead of stitching together multiple commands.
3. Confirm older explicit commands still behave correctly where they remain supported.

**Done When:**
- The docs point users/agents at the simpler path.
- Tests cover both the happy path and lifecycle guardrails.

## Final Verification
- `cargo test -p orbit-core`
- `cargo test -p orbit --test task_commands`
- `orbit task --help`