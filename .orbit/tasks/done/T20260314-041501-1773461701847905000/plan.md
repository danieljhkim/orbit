# Legacy SQLite Cleanup Plan

**Goal:** Leave SQLite support only where Orbit still explicitly wants it: agent sessions, tools, and audit events.
**Scope:** Remove the remaining legacy/general-purpose SQLite layer, narrow the surviving SQLite bootstrap, and rewire runtime callers away from deleted stores.
**Assumptions:** `audit_store`, memo storage, and lock storage should no longer be first-class SQLite responsibilities after this refactor.
**Risks:** Some runtime features may still be coupled to these legacy SQLite stores and will need replacement implementations or API reshaping.

## Task 1: Define and enforce the new SQLite boundary

**Files:**
- Modify: `orbit-store/src/lib.rs`
- Modify: `orbit-store/src/backend/factory.rs`
- Modify: `orbit-store/src/backend/sqlite_backends.rs`
- Modify: `orbit-store/src/sqlite/mod.rs`
- Modify: `orbit-core/src/runtime/builder.rs`
- Modify: `orbit-core/src/context.rs`

**Steps:**
1. Remove exports and backend adapters for SQLite-backed responsibilities outside `agent_session`, `tools`, and `audit_events`.
2. Update runtime/context wiring so deleted SQLite stores are no longer constructed or required.
3. Introduce any minimal non-SQLite replacements needed for still-required runtime behavior.

**Done When:**
- The codebase has an explicit, narrow SQLite boundary.
- Runtime initialization no longer depends on removed SQLite services.

## Task 2: Delete legacy SQLite stores and shared scaffolding

**Files:**
- Modify: `orbit-store/src/sqlite/connection.rs`
- Modify: `orbit-store/src/sqlite/migration.rs`
- Modify: `orbit-store/src/sqlite/memo_store.rs`
- Modify: `orbit-store/src/sqlite/audit_store.rs`
- Modify: `orbit-store/src/sqlite/lock.rs`
- Modify: `orbit-store/migrations/0001_init.sql`

**Steps:**
1. Remove unused SQLite modules and APIs that fall outside the keep-list.
2. Trim `migration.rs` and the seed SQL so they only initialize schema needed by surviving SQLite stores.
3. Simplify shared connection/transaction code so it reflects the reduced responsibility set.

**Done When:**
- `memo_store.rs` and other removed legacy SQLite modules are gone or fully retired.
- SQLite bootstrap no longer creates unrelated legacy tables.

## Task 3: Migrate callers, commands, and tests off removed SQLite behavior

**Files:**
- Modify: `orbit-core/src/runtime/mod.rs`
- Modify: `orbit-core/src/runtime/mutation.rs`
- Modify: `orbit-core/src/job/job.rs`
- Modify: `orbit-core/src/command/agent.rs`
- Modify: `orbit-cli`/`orbit-core`/`orbit-store` tests that still depend on removed SQLite stores

**Steps:**
1. Replace or remove uses of deleted SQLite-backed audit, lock, or memo behavior.
2. Rework tests so they validate the new boundary rather than the legacy SQLite surface.
3. Confirm user-facing behavior still works for the kept SQLite-backed features.

**Done When:**
- No production caller references removed SQLite stores.
- Tests cover the supported SQLite boundary and no longer depend on legacy SQLite plumbing.

## Final Verification
- `cargo test -p orbit-store`
- `cargo test -p orbit-core`
- `cargo test -p orbit`
- `cargo test --workspace`