# Shorter Ordered ID Implementation Plan

**Goal:** Introduce shorter task and job-run IDs without breaking creation ordering or runtime safety.
**Scope:** Task ID generation, job-run ID generation, collision handling, and test/doc updates directly related to those formats.
**Assumptions:** Existing records remain valid; new IDs only affect newly created tasks and job runs.
**Risks:** Shared ID helpers may accidentally change job IDs too; second-level timestamps require explicit collision handling.

## Task 1: Audit current generators and isolate scope

**Files:**
- Modify: `orbit-store/src/file/task_store.rs`
- Modify: `orbit-store/src/file/job_store.rs`
- Modify: `orbit-store/src/lib.rs`
- Modify: `orbit-store/src/sqlite/job_store.rs`

**Steps:**
1. Identify every current caller that relies on nanosecond-based IDs.
2. Decide whether to split shared helpers so job IDs remain unchanged unless intentionally included.
3. Document the exact collision strategy for file-backed tasks and job runs.

**Done When:**
- The implementation path is clear and does not accidentally widen the ID-format change.

## Task 2: Implement shorter task and job-run ID formats

**Files:**
- Modify: `orbit-store/src/file/task_store.rs`
- Modify: `orbit-store/src/file/job_store.rs`
- Modify: `orbit-store/src/lib.rs`
- Modify: `orbit-store/src/sqlite/job_store.rs`

**Steps:**
1. Update task ID generation to `TYYYYMMDD-HHMMSS-<suffix>` using a short sequence or random digit block.
2. Update job-run ID generation to `JRYYYYMMDD-HHMMSS-<suffix>` using a short sequence or random digit block.
3. Preserve clear creation-time ordering via the visible timestamp portion and safe collision handling.
4. Ensure SQLite-backed job-run insertion remains safe under same-second collisions.
5. Confirm existing task and job-run lookup behavior still accepts older IDs.

**Done When:**
- New tasks and job runs use the shorter formats and collisions are handled safely.

## Task 3: Update tests and user-facing expectations

**Files:**
- Modify: `orbit-cli/tests/task_commands.rs`
- Modify: `orbit-cli/tests/job_commands.rs`
- Modify: any additional tests or docs that assert concrete ID values

**Steps:**
1. Replace brittle long-form ID expectations with assertions aligned to the new formats.
2. Add coverage for same-second collisions for tasks and job runs where practical.
3. Run targeted tests for task creation and job-run flows.

**Done When:**
- Tests validate the new formats and collision behavior.

## Final Verification
- `cargo test -p orbit-cli task_commands`
- `cargo test -p orbit-cli job_commands`
- Run any targeted orbit-store tests covering file and SQLite job-run creation