# Refresh Bundled Defaults On Init

**Goal:** Make explicit `orbit init` initialize and refresh the full bundled set of default activities and jobs.
**Scope:** `orbit-core` init/seeding logic plus regression coverage in `orbit-core` and `orbit-cli`.
**Assumptions:** Explicit `orbit init` should refresh bundled defaults in place, while implicit bootstrap paths should remain conservative and avoid overwriting local customizations unless they are clearly built-in defaults.
**Risks:** Overwriting tracked runtime copies too aggressively could clobber local edits; the refresh logic must be limited to bundled built-in IDs and preserve non-default artifacts.

## Task 1: Align bundled default enumerations with assets

**Files:**
- Modify: `orbit-core/src/command/activity.rs`
- Modify: `orbit-core/src/command/job.rs`
- Review: `orbit-core/assets/activities/*.yaml`
- Review: `orbit-core/assets/jobs/*.yaml`

**Steps:**
1. Replace the stale hard-coded default activity list so it includes all bundled activity assets currently shipped in `orbit-core/assets/activities`.
2. Verify the bundled job list matches the shipped job assets under `orbit-core/assets/jobs`.
3. Add or tighten parse/coverage tests so future asset additions fail loudly if the init seed list drifts again.

**Done When:**
- explicit defaults enumeration matches the bundled activities/jobs shipped in the repo
- tests would catch future count/name drift

## Task 2: Refresh built-in activities and jobs on explicit init

**Files:**
- Modify: `orbit-core/src/command/init.rs`
- Modify: `orbit-core/src/command/activity.rs`
- Modify: `orbit-core/src/command/job.rs`
- Modify tests near init behavior in `orbit-cli/tests/init_commands.rs`

**Steps:**
1. Thread explicit-init refresh intent into activity/job seeding the same way identities and skills already receive `overwrite` behavior.
2. Refresh built-in activity/job files when `orbit init` is run explicitly, but keep implicit bootstrap behavior non-destructive.
3. Ensure refresh only applies to bundled built-in IDs so custom activities/jobs are left alone.
4. Preserve the existing legacy rename migration behavior before refresh runs.

**Done When:**
- explicit `orbit init` updates existing built-in activity/job definitions to match bundled assets
- implicit initialization still only fills in missing defaults
- non-default runtime artifacts are untouched

## Task 3: Prove init creates and refreshes the full default set

**Files:**
- Modify/add tests in `orbit-core` and `orbit-cli/tests/init_commands.rs`

**Steps:**
1. Add a regression test that a fresh init produces all 10 bundled activities and all 4 bundled jobs.
2. Add a refresh test that mutates a built-in activity/job file in a temp workspace, reruns explicit `orbit init`, and verifies the bundled content is restored.
3. Keep assertions focused on user-visible behavior, not implementation details.

**Done When:**
- fresh init coverage proves the full bundled defaults appear
- rerunning explicit init proves built-in activities/jobs are refreshed from assets

## Final Verification
- `cargo test -p orbit-core`
- `cargo test -p orbit-cli --test init_commands`
- manual `orbit init` in a temp workspace followed by `orbit activity list --json` and `orbit job list --json` spot checks if needed