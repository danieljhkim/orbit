# Identity Consolidation Implementation Plan

**Goal:** Replace the six default identities with four canonical personas while keeping runtime loading, seeded defaults, tracked repo-local copies, and built-in activity references consistent.
**Scope:** Identity roles, default identity assets, tracked `.orbit` identity/activity copies, seed-default wiring, and tests that assert the default identity set.
**Assumptions:** Historical task records that mention old identities are audit artifacts and should not be rewritten. `steve` remains `ceo`; only `architect` and `reviewer` need new runtime role support.
**Risks:** Missing one of the tracked `.orbit` or built-in activity references would leave Orbit in a split-brain state where init, runtime, and checked-in defaults disagree.

## Task 1: Add Runtime Support For New Identity Roles

**Files:**
- Modify: `orbit-types/src/identity.rs`
- Modify: `orbit-cli/src/command/identity.rs`
- Test: `orbit-cli/tests/identity_commands.rs`

**Steps:**
1. Extend `IdentityRole` to support `architect` and `reviewer` while preserving existing roles and external string formatting.
2. Update any user-facing role help text that enumerates supported values.
3. Add or adjust tests for parsing/display and CLI role filtering with the new roles.
4. Run targeted tests for identity role parsing and CLI identity commands.

**Done When:** Orbit can parse, display, and filter `architect` and `reviewer` identities without regressing existing roles.

## Task 2: Replace The Default Identity Set In Both Sources Of Truth

**Files:**
- Delete: `orbit-core/assets/identities/grace.yaml`
- Delete: `orbit-core/assets/identities/john.yaml`
- Delete: `orbit-core/assets/identities/kent.yaml`
- Delete: `orbit-core/assets/identities/rob.yaml`
- Create: `orbit-core/assets/identities/linus.yaml`
- Create: `orbit-core/assets/identities/lamport.yaml`
- Modify: `orbit-core/assets/identities/prii.yaml`
- Modify: `orbit-core/assets/identities/steve.yaml` only if minor cleanup is needed
- Delete: `.orbit/identities/grace.yaml`
- Delete: `.orbit/identities/john.yaml`
- Delete: `.orbit/identities/kent.yaml`
- Delete: `.orbit/identities/rob.yaml`
- Create: `.orbit/identities/linus.yaml`
- Create: `.orbit/identities/lamport.yaml`
- Modify: `.orbit/identities/prii.yaml`
- Modify: `.orbit/identities/steve.yaml` only if kept in sync with asset cleanup

**Steps:**
1. Create `linus` as the pragmatic engineer and `lamport` as the specification-first architect.
2. Update `prii` from maintainer/leader framing to reviewer framing with role `reviewer`.
3. Keep `steve` as `ceo`; clean up wording only if needed for consistency.
4. Mirror the same four-file set in both `orbit-core/assets/identities` and tracked `.orbit/identities`.
5. Confirm there is no stray appended content or malformed YAML in the surviving files.

**Done When:** Both tracked identity directories contain exactly `linus`, `lamport`, `prii`, and `steve`, and each file parses under the updated role model.

## Task 3: Update Seeded Defaults And Activity References

**Files:**
- Modify: `orbit-core/src/command/identity.rs`
- Modify: `orbit-core/assets/activities/resolve-backlogged-task.yaml`
- Modify: `.orbit/activities/active/resolve-backlogged-task.yaml`
- Inspect/modify if needed: `orbit-core/assets/activities/approve-task-leader.yaml`
- Inspect/modify if needed: `orbit-core/assets/activities/perform-maintenance.yaml`
- Inspect/modify if needed: `orbit-core/assets/activities/triage-and-dispatch-task.yaml`
- Inspect/modify if needed: `orbit-core/assets/activities/oversee-orbit-operations.yaml`
- Inspect/modify if needed: matching tracked `.orbit/activities/active/*.yaml` copies

**Steps:**
1. Replace the hardcoded six-entry default seed list with the new four identities.
2. Repoint `resolve-backlogged-task` from `kent` to the canonical execution identity (`linus`).
3. Audit the other built-in activity identity references and ensure they still resolve under the new default set.
4. Keep tracked `.orbit/activities/active` copies aligned with the built-in asset references where those copies are versioned in the repo.

**Done When:** `orbit init` would seed the four canonical identities, and no tracked built-in activity still references a retired identity.

## Task 4: Refresh Default-Set Tests And Verification

**Files:**
- Modify: `orbit-cli/tests/init_commands.rs`
- Modify: `orbit-cli/tests/identity_commands.rs`
- Inspect/modify as needed: `orbit-core/tests/job_runtime_behavior.rs`

**Steps:**
1. Update init tests that currently assert `john/kent/rob/grace` exist after seeding.
2. Add coverage that the new roles and default identities are visible through CLI surfaces.
3. Add or update any runtime tests needed if role parsing or identity resolution assumptions changed.
4. Run focused tests, then a broader workspace build.

**Done When:** Tests and verification commands reflect the four-identity default set instead of the retired six.

## Final Verification
- `cargo test -p orbit-cli identity_commands init_commands`
- `cargo test -p orbit-core`
- `cargo build --workspace`
- `orbit identity list`
- `orbit identity show linus`
- `orbit identity show lamport`
- `orbit identity show prii`
- `orbit identity show steve`
- `rg -n "grace|john|kent|rob" orbit-core/assets/identities orbit-core/assets/activities .orbit/identities .orbit/activities/active`