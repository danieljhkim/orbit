# Identity Resolution Defect Plan

**Goal:** Restore successful `approve-task-leader` job execution by fixing the runtime path that fails to resolve identity `prii`.
**Scope:** Identity root selection, activity identity resolution, approval-job execution path, and targeted regression coverage.
**Assumptions:** The configured identity `prii` is valid and should resolve from the repo-local Orbit state.
**Risks:** The root cause may be shared with broader repo-root vs home-root behavior, so the fix must not create a second inconsistent identity path.

## Task 1: Reproduce and isolate the failing identity lookup

**Files:**
- Review: `.orbit/jobs/runs/job-approve-task-leader/jrun-1773289488943420000.yaml`
- Review: `.orbit/activities/active/approve-task-leader.yaml`
- Review: `.orbit/identities/prii.yaml`
- Modify: `orbit-core/src/runtime/builder.rs`
- Modify: `orbit-core/src/config/runtime.rs`
- Modify: `orbit-store/src/file/identity_store.rs`

**Steps:**
1. Trace the identity root used by the runtime for this job execution.
2. Confirm whether the runtime is pointing at the wrong identity directory or failing to seed/load repo-local identities.
3. Identify the narrowest fix that restores `prii` resolution for activity execution.

**Done When:**
- The specific cause of `identity not found: prii` is understood and localized.

## Task 2: Implement the fix and preserve validation

**Files:**
- Modify: the runtime/config/activity code identified in Task 1
- Modify: any seeded/default data only if required by the confirmed root cause

**Steps:**
1. Implement the fix so `approve-task-leader` can resolve `prii` successfully.
2. Keep identity validation strict; do not silently ignore missing identities.
3. Verify the approval activity still carries the intended identity after the change.

**Done When:**
- The approval job can run without failing on `identity not found: prii`.

## Task 3: Add regression coverage

**Files:**
- Modify: `orbit-cli/tests/init_commands.rs`
- Modify: `orbit-core/tests/job_runtime_behavior.rs`
- Modify: any additional focused identity/runtime tests needed for the fix

**Steps:**
1. Add a test that would have caught the `approve-task-leader` identity-resolution failure.
2. Add or update coverage for repo-local identity availability during job execution.
3. Re-run the targeted tests covering job execution and identity/bootstrap behavior.

**Done When:**
- The defect is covered by automated tests and does not regress silently.

## Final Verification
- Run the targeted tests that cover the chosen fix path
- Re-run or inspect `job-approve-task-leader` execution to confirm the identity error is gone