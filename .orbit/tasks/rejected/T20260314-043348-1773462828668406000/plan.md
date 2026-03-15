# Auto-Commit Removal Plan

**Goal:** Remove Orbit's automatic git commit flow for job-created artifacts and leave a simpler, non-mutating job completion path.
**Scope:** Runtime commit logic, the `created_file` result contract, built-in activity/skill docs, and test coverage tied to the feature.
**Assumptions:** Built-in jobs can continue to produce reports as plain artifacts without Orbit committing them.
**Risks:** Some operational jobs may implicitly depend on committed reports, and the protocol cleanup may touch multiple shared docs/assets.

## Task 1: Remove runtime auto-commit execution

**Files:**
- Modify: `orbit-core/src/command/job.rs`
- Modify: related runtime/helper modules if they reference the auto-commit path
- Modify: any shared types or constants only used by this feature

**Steps:**
1. Remove the success-path hook that invokes auto-commit after job-run persistence.
2. Delete the helper that stages/commits the created file and run artifact.
3. Remove obsolete validation, error codes, or protocol fields that only exist for this flow.

**Done When:**
- Successful job execution no longer triggers any automatic git mutations.
- No dead runtime code remains for the removed auto-commit behavior.

## Task 2: Clean up the job/activity contract and documentation

**Files:**
- Modify: `orbit-core/assets/activities/perform-maintenance.yaml`
- Modify: `orbit-core/assets/activities/oversee-orbit-operations.yaml`
- Modify: `orbit-core/assets/skills/orbit-maintain-system/SKILL.md`
- Modify: `orbit-core/assets/skills/orbit-operations-management/SKILL.md`
- Modify: mirrored `.orbit/` activity or skill assets if they are still source-of-truth in this repo

**Steps:**
1. Remove documentation that instructs agents to return report files for Orbit to auto-commit.
2. Update schemas/contracts so built-in activities no longer promise automatic git commits.
3. Keep any remaining artifact/report instructions aligned with the new non-commit behavior.

**Done When:**
- No built-in Orbit docs or activity schemas claim that Orbit auto-commits created files.
- The remaining result contract is internally consistent.

## Task 3: Replace feature-specific tests with boundary checks

**Files:**
- Modify: `orbit-core/tests/job_runtime_behavior.rs`
- Modify: any CLI/core/store tests that assume committed reports or run artifacts

**Steps:**
1. Remove or rewrite tests that assert automatic commits happen.
2. Add targeted coverage proving successful runs do not trigger git commits as a side effect.
3. Re-run the affected test suites.

**Done When:**
- Test coverage reflects the simpler non-auto-commit runtime behavior.
- No test still depends on auto-created git commits.

## Final Verification
- `cargo test -p orbit-core --test job_runtime_behavior`
- `cargo test -p orbit-core`
- `cargo test -p orbit`