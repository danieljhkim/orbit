# Task Identity Consistency Plan

**Goal:** Make task attribution fields consistent with the Orbit identity catalog so assignment, creation, proposal, approval, and rejection metadata all mean the same thing.
**Scope:** Task CLI, runtime validation, task data model/storage, and task workflow docs/tests.
**Assumptions:** Orbit should have a canonical concept of task actors rather than mixing identity-backed and free-form attribution.
**Risks:** Existing task bundles may contain legacy free-form names, and the final contract must balance strictness with backward compatibility.

## Task 1: Define the task attribution contract

**Files:**
- Modify: `orbit-types/src/task.rs`
- Modify: `orbit-core/src/command/task.rs`
- Modify: `orbit-cli/src/command/task.rs`
- Modify: task-related workflow docs/skills if they need contract updates

**Steps:**
1. Decide whether task actor fields should store canonical identity IDs, display names, or both.
2. Update the shared task/runtime command surface to reflect that decision consistently.
3. Add failing tests that demonstrate the chosen behavior for assignment and approval fields.

**Done When:**
- Orbit has one coherent contract for task actor/assignee fields.
- The CLI and runtime no longer disagree about which fields are identity-backed.

## Task 2: Implement validation and storage coherently

**Files:**
- Modify: `orbit-core/src/command/task.rs`
- Modify: `orbit-store/src/file/task_store.rs`
- Modify: any task serialization/deserialization code needed for compatibility

**Steps:**
1. Add validation or structured storage for task identity-bearing fields.
2. Preserve compatibility for existing persisted tasks where practical.
3. Ensure task history/comments/decision metadata still remain readable to humans.

**Done When:**
- Task add/update/approve/reject flows enforce the chosen identity contract.
- Existing tasks can still be loaded safely.

## Task 3: Update CLI/docs/tests around the new contract

**Files:**
- Modify: `orbit-cli/tests/task_commands.rs`
- Modify: `orbit-core/assets/skills/orbit-create-task/SKILL.md`
- Modify: `orbit-core/assets/skills/orbit-approve-task/SKILL.md`
- Modify: mirrored `.orbit` skill copies if they remain part of the shipped workflow surface

**Steps:**
1. Update CLI help/docs so identity-backed flags are explicit.
2. Add coverage for valid and invalid identity inputs in task lifecycle commands.
3. Verify the common Orbit workflow remains ergonomic.

**Done When:**
- Task docs match the enforced behavior.
- Tests cover both acceptance and rejection paths for task attribution inputs.

## Final Verification
- `cargo test -p orbit-core`
- `cargo test -p orbit --test task_commands`
- `cargo test -p orbit`