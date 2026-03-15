# Exception-Safe Job Run Finalization Plan

**Goal:** Ensure a job run is finalized to `failed` whenever execution errors after the run has already been marked `running`.
**Scope:** Job execution orchestration in `orbit-core`, especially pre-step setup and step bookkeeping paths that can currently return early.
**Assumptions:** The main bug is an exception path after `mark_job_run_running_backend()` and before `finalize_job_run_backend()`, not just missing process liveness checks.
**Risks:** The fix needs to preserve accurate error reporting without masking the original failure reason or double-finalizing runs.

## Task 1: Trace all post-running early-return paths

**Files:**
- Modify: `orbit-core/src/command/job.rs`
- Read/Modify: `orbit-core/src/command/job_run.rs` if recovery behavior needs follow-up alignment

**Steps:**
1. Inspect `execute_activity_with_retries()` starting at the point where a run is marked `running`.
2. Identify every fallible path that can return before `finalize_job_run_backend()` is called.
3. Confirm which path produced the observed `jrun-20260315-212834` shape (`running` with `steps: []`).

**Done When:**
- All exception paths that can orphan a `running` job run are identified.

## Task 2: Make job execution failure-safe after run start

**Files:**
- Modify: `orbit-core/src/command/job.rs`
- Modify any directly affected runtime/store helpers if needed

**Steps:**
1. Restructure execution so that once a run is marked `running`, later errors finalize it to `failed` before returning.
2. Preserve the original error context in the stored run state and returned error.
3. Ensure pre-step errors (for example schema validation or execution-context construction failures) still leave a useful failure record even when no step completes.

**Done When:**
- No post-start exception path leaves a run stuck in `running`.
- Failed runs clearly record the triggering error.

## Task 3: Add regression coverage for orphaned-running prevention

**Files:**
- Modify: `orbit-core/tests/job_runtime_behavior.rs`

**Steps:**
1. Add a test that reproduces a failure after `running` is set but before the first step completes.
2. Assert the run becomes `failed` rather than remaining `running`.
3. Keep existing stale-timeout recovery coverage intact as separate secondary protection.

**Done When:**
- Tests prove the runtime no longer leaves exception-orphaned runs in `running`.

## Final Verification
- `cargo test -p orbit-core --test job_runtime_behavior`
- `cargo test --workspace`