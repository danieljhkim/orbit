# Job and Activity SQLite Removal Plan

**Goal:** Remove SQLite as a supported persistence option for jobs and activities while preserving audit-related SQLite behavior.
**Scope:** Config, runtime wiring, job/activity store adapters, schema bootstrap, and tests/docs that currently preserve job/activity SQLite parity.
**Assumptions:** Audit and audit-event stores remain SQLite-backed after this change.
**Risks:** In-memory runtime helpers and regression tests may depend on SQLite job/activity stores today and will need careful replacement.

## Task 1: Remove config and runtime support for SQLite-backed jobs and activities

**Files:**
- Modify: `orbit-core/src/config/persistence.rs`
- Modify: `orbit-core/src/runtime/builder.rs`
- Modify: `orbit-core/assets/config/default-config.toml`
- Modify: `orbit-core/assets/config/default-config-repo.toml`
- Modify: `orbit-cli/tests/config_commands.rs`

**Steps:**
1. Remove SQLite as an accepted persistence type for `job` and `activity` config.
2. Update runtime builder code so jobs and activities are always constructed from supported non-SQLite backends.
3. Adjust config docs/tests/defaults so SQLite is no longer advertised for jobs or activities.

**Done When:**
- Config rejects SQLite for jobs and activities.
- Runtime initialization no longer branches to SQLite-backed job/activity stores.

## Task 2: Remove job/activity SQLite backend implementations

**Files:**
- Modify: `orbit-store/src/backend/factory.rs`
- Modify: `orbit-store/src/backend/sqlite_backends.rs`
- Modify: `orbit-store/src/lib.rs`
- Modify: `orbit-store/src/sqlite/job_store.rs`
- Modify: `orbit-store/src/sqlite/activity_store.rs`

**Steps:**
1. Stop exporting SQLite-backed job/activity store constructors from the store layer.
2. Remove or retire SQLite job/activity backend implementations that are no longer reachable.
3. Keep shared store APIs coherent after the job/activity SQLite paths are deleted.

**Done When:**
- No supported store factory path creates SQLite-backed jobs or activities.
- The codebase no longer carries dead SQLite job/activity adapter code.

## Task 3: Trim SQLite schema/tests to audit-focused responsibilities

**Files:**
- Modify: `orbit-store/src/sqlite/migration.rs`
- Modify: `orbit-store/migrations/0001_init.sql`
- Modify: `orbit-core/tests/job_runtime_behavior.rs`
- Modify: relevant store/runtime tests that currently exercise SQLite-backed jobs or activities

**Steps:**
1. Remove job/activity-specific SQLite schema bootstrap or migration logic that is no longer needed.
2. Rewrite job/activity SQLite tests to use supported paths or validate config rejection instead.
3. Verify remaining SQLite responsibilities are limited to audit-related persistence.

**Done When:**
- SQLite bootstrap/migration logic no longer exists solely to support jobs or activities.
- Tests reflect the new boundary: file-backed jobs/activities, SQLite-backed audit only.

## Final Verification
- `cargo test -p orbit-core`
- `cargo test -p orbit-store`
- `cargo test -p orbit-cli config_commands -- --nocapture`
- `cargo test --workspace`