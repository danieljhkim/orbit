use std::path::Path;

use orbit_common::types::OrbitError;
use rusqlite::Connection;

mod ledger;

pub use ledger::{AppliedMigration, SUPPORTED_SCHEMA_VERSION};
pub(crate) use ledger::{applied_migrations, current_schema_version};

/// Bring the store database up to the newest supported schema version,
/// applying any pending versioned migrations (each transactional and
/// recorded in the `schema_meta` ledger). Refuses to open a database whose
/// recorded schema version is newer than this binary supports.
pub(crate) fn apply_schema(conn: &Connection) -> Result<(), OrbitError> {
    ledger::run_migrations(conn, ledger::MIGRATIONS)
}

/// Registry metadata for one schema migration not yet recorded as applied,
/// as surfaced by `orbit migrate --dry-run` (ORB-10012).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSchemaMigration {
    pub version: u32,
    pub name: &'static str,
}

/// Read-only view of a store database's migration ledger: the recorded
/// schema version plus the registry migrations still pending against it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaLedgerStatus {
    /// Schema version recorded in the ledger (0 for a fresh/pre-ledger or
    /// nonexistent database).
    pub current_version: u32,
    /// Registry migrations newer than `current_version`, in apply order.
    /// Empty when the database is current or newer than this binary
    /// (compare against [`SUPPORTED_SCHEMA_VERSION`] to distinguish).
    pub pending: Vec<PendingSchemaMigration>,
}

/// Inspect the migration ledger of the store database at `db_path` without
/// opening it for writing — and therefore without triggering the automatic
/// migrations that [`crate::Store::open`] applies. A missing database reads
/// as version 0 with every registry migration pending (opening it would
/// create and migrate it). Powers `orbit migrate --dry-run`.
pub fn read_schema_ledger_status(db_path: &Path) -> Result<SchemaLedgerStatus, OrbitError> {
    let current_version = if db_path.exists() {
        let conn = Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| {
            OrbitError::Store(format!(
                "cannot open store database '{}' read-only: {e}",
                db_path.display()
            ))
        })?;
        current_schema_version(&conn)?
    } else {
        0
    };

    Ok(SchemaLedgerStatus {
        current_version,
        pending: pending_schema_migrations_after(current_version),
    })
}

/// Registry migrations newer than `current_version`, in apply order. Lets
/// callers that already know the recorded version (e.g. via
/// [`crate::Store::schema_version`]) list what is pending without another
/// database open.
pub fn pending_schema_migrations_after(current_version: u32) -> Vec<PendingSchemaMigration> {
    ledger::MIGRATIONS
        .iter()
        .filter(|m| m.version > current_version)
        .map(|m| PendingSchemaMigration {
            version: m.version,
            name: m.name,
        })
        .collect()
}

/// v1 `baseline` migration: the full pre-ledger idempotent schema. Safe to
/// run on both a fresh database and any legacy database created by the
/// pre-versioning migration code, which is how existing databases adopt
/// the versioned ledger.
fn apply_baseline_schema(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
            CREATE TABLE IF NOT EXISTS tools (
                name TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                parameters_json TEXT NOT NULL DEFAULT '[]',
                enabled INTEGER NOT NULL DEFAULT 1,
                builtin INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS agent_sessions (
                session_id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL,
                identity_id TEXT,
                identity_name TEXT,
                identity_role TEXT,
                identity_block TEXT,
                skill_names TEXT NOT NULL,
                composed_context_hash TEXT NOT NULL,
                effective_allowed_tools TEXT NOT NULL,
                tool_calls TEXT NOT NULL,
                outcome TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS audit_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                execution_id TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                command TEXT NOT NULL,
                subcommand TEXT,
                tool_name TEXT,
                target_type TEXT,
                target_id TEXT,
                role TEXT NOT NULL,
                status TEXT NOT NULL,
                exit_code INTEGER NOT NULL,
                duration_ms INTEGER NOT NULL,
                working_directory TEXT NOT NULL,
                arguments_json TEXT,
                stdout_truncated TEXT,
                stderr_truncated TEXT,
                error_message TEXT,
                host TEXT,
                pid INTEGER NOT NULL,
                session_id TEXT,
                task_id TEXT,
                job_run_id TEXT,
                activity_id TEXT,
                step_index INTEGER
            );

            CREATE TABLE IF NOT EXISTS task_reservations (
                reservation_id TEXT PRIMARY KEY,
                workspace_orbit_dir TEXT NOT NULL,
                workspace_id TEXT,
                task_ids_json TEXT NOT NULL,
                files_json TEXT NOT NULL,
                actor TEXT NOT NULL,
                created_at TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                released_at TEXT,
                owner_run_id TEXT,
                owner_metadata_json TEXT,
                release_reason TEXT,
                release_metadata_json TEXT
            );

            CREATE TABLE IF NOT EXISTS invocations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts TEXT NOT NULL,
                job_run_id TEXT NOT NULL,
                activity_id TEXT NOT NULL,
                agent TEXT NOT NULL,
                model TEXT,
                slot TEXT,
                duration_ms INTEGER NOT NULL DEFAULT 0,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                cache_create_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                tool_call_count INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS invocation_tasks (
                invocation_id INTEGER NOT NULL,
                task_id TEXT NOT NULL,
                PRIMARY KEY(invocation_id, task_id),
                FOREIGN KEY(invocation_id) REFERENCES invocations(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS tool_calls (
                invocation_id INTEGER NOT NULL,
                seq INTEGER NOT NULL,
                tool_name TEXT NOT NULL,
                result_bytes INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(invocation_id, seq),
                FOREIGN KEY(invocation_id) REFERENCES invocations(id) ON DELETE CASCADE
            );

            -- ADR envelope index. Bodies live on disk under <root>/<state>/<id>/body.md;
            -- this table indexes the YAML envelope fields for filter queries.
            -- Arrays (related_features, related_tasks, tags, paths, legacy_ids,
            -- supersedes, validation_warnings) are stored as JSON-encoded strings
            -- so filters can use `LIKE '%"<value>"%'` until the corpus warrants junction
            -- tables. FTS5 over body content is owned by `orbit-search::vector`,
            -- not this schema.
            CREATE TABLE IF NOT EXISTS adrs (
                id TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                title TEXT NOT NULL,
                owner TEXT NOT NULL,
                related_features TEXT NOT NULL DEFAULT '[]',
                related_tasks TEXT NOT NULL DEFAULT '[]',
                tags TEXT NOT NULL DEFAULT '[]',
                paths TEXT NOT NULL DEFAULT '[]',
                legacy_ids TEXT NOT NULL DEFAULT '[]',
                supersedes TEXT NOT NULL DEFAULT '[]',
                superseded_by TEXT,
                validation_warnings TEXT NOT NULL DEFAULT '[]',
                legacy_validation TEXT NOT NULL DEFAULT 'none',
                created_at TEXT NOT NULL,
                accepted_at TEXT,
                last_updated TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_adrs_status ON adrs(status);
            CREATE INDEX IF NOT EXISTS idx_adrs_owner ON adrs(owner);
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    ensure_agent_sessions_schema(conn)?;
    ensure_tools_schema(conn)?;
    ensure_adr_index_schema(conn)?;
    ensure_audit_events_schema(conn)?;
    ensure_task_reservations_schema(conn)?;
    ensure_learning_index_schema(conn)?;
    ensure_invocation_schema(conn)?;
    ensure_v2_state_consolidation_schema(conn)?;
    ensure_routine_schema(conn)?;

    Ok(())
}

/// v2 `learnings_index_workspace_scope` migration (ORB-10113): re-key the
/// learning envelope index by `(workspace_id, id)`.
///
/// The index previously had no workspace discriminator and was keyed only by
/// learning ID, so in the shared host-global database rows written by one
/// workspace leaked into another workspace's searches and reminders. YAML
/// under each `.orbit/learnings/` is the source of truth, and legacy rows
/// cannot be attributed to a workspace reliably, so this migration discards
/// every indexed row and lets each runtime rebuild its own rows from YAML via
/// `sync_learnings`. It touches only SQLite — no `learning.yaml` file is read
/// or modified.
fn apply_learning_index_workspace_scope(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
            DROP TABLE IF EXISTS learnings_index;

            CREATE TABLE learnings_index (
                workspace_id TEXT NOT NULL,
                id           TEXT NOT NULL,
                status       TEXT NOT NULL,
                paths        TEXT NOT NULL,
                tags         TEXT NOT NULL,
                summary      TEXT NOT NULL,
                updated_at   TEXT NOT NULL,
                priority     INTEGER,
                PRIMARY KEY (workspace_id, id)
            );

            CREATE INDEX IF NOT EXISTS learnings_active
                ON learnings_index(workspace_id, status) WHERE status = 'active';
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))
}

fn ensure_routine_schema(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
            -- Host-local routine scheduler state [ORB-10021]. Lives only in
            -- the host-global store database and is never synced between
            -- hosts (ADR-0208); routine *definitions* are git-versioned YAML
            -- in routine-source workspaces.

            -- Per-routine cursor: first observation baseline + last slot
            -- consumed. A routine never fires for slots before its baseline.
            CREATE TABLE IF NOT EXISTS routine_cursors (
                routine_name TEXT PRIMARY KEY,
                baseline_at TEXT NOT NULL,
                last_slot TEXT,
                updated_at TEXT NOT NULL
            );

            -- One row per fire attempt. The (name, slot, attempt) uniqueness
            -- is the idempotency key that prevents double fires for the same
            -- scheduled slot across overlapping or crashed sweeps.
            CREATE TABLE IF NOT EXISTS routine_fires (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                routine_name TEXT NOT NULL,
                slot TEXT NOT NULL,
                attempt INTEGER NOT NULL DEFAULT 1,
                state TEXT NOT NULL,
                run_id TEXT,
                source_workspace TEXT NOT NULL,
                detail TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                UNIQUE(routine_name, slot, attempt)
            );

            CREATE INDEX IF NOT EXISTS idx_routine_fires_name_slot
            ON routine_fires(routine_name, slot DESC, attempt DESC);

            CREATE INDEX IF NOT EXISTS idx_routine_fires_state
            ON routine_fires(state);

            -- Host-local suppressions written by `orbit routine pause`;
            -- durable across reboots, invisible to git.
            CREATE TABLE IF NOT EXISTS routine_pauses (
                routine_name TEXT PRIMARY KEY,
                paused_at TEXT NOT NULL,
                actor TEXT
            );
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))
}

fn ensure_adr_index_schema(conn: &Connection) -> Result<(), OrbitError> {
    add_column_if_missing(
        conn,
        "ALTER TABLE adrs ADD COLUMN tags TEXT NOT NULL DEFAULT '[]'",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE adrs ADD COLUMN paths TEXT NOT NULL DEFAULT '[]'",
    )
}

fn ensure_learning_index_schema(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
            -- Project-learnings envelope index. YAML records live on disk under
            -- `<root>/<id>/learning.yaml`; status lives in the YAML body.
            -- this table indexes the envelope fields for fast scope-glob
            -- lookups. Arrays are stored as JSON strings for the same reason
            -- the ADR index does it: phase-1 corpora are small and a junction
            -- table is overkill. Per ADR-004, ranking and FTS over body
            -- content are deferred to phase 2.
            CREATE TABLE IF NOT EXISTS learnings_index (
                id          TEXT PRIMARY KEY,
                status      TEXT NOT NULL,
                paths       TEXT NOT NULL,
                tags        TEXT NOT NULL,
                summary     TEXT NOT NULL,
                updated_at  TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS learnings_active
                ON learnings_index(status) WHERE status = 'active';
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    // C2 (T20260511-6) adds an optional `priority` column used as the
    // secondary ranking key in `search`. NULL is acceptable; the search
    // path orders Some(N) ahead of None and falls back to updated_at.
    add_column_if_missing(
        conn,
        "ALTER TABLE learnings_index ADD COLUMN priority INTEGER",
    )?;

    Ok(())
}

fn ensure_agent_sessions_schema(conn: &Connection) -> Result<(), OrbitError> {
    if table_exists(conn, "agent_sessions")?
        && table_has_foreign_key_to(conn, "agent_sessions", "tasks")?
    {
        conn.execute_batch(
            r#"
                ALTER TABLE agent_sessions RENAME TO agent_sessions_legacy;

                CREATE TABLE agent_sessions (
                    session_id TEXT PRIMARY KEY,
                    task_id TEXT NOT NULL,
                    identity_id TEXT,
                    identity_name TEXT,
                    identity_role TEXT,
                    identity_block TEXT,
                    skill_names TEXT NOT NULL,
                    composed_context_hash TEXT NOT NULL,
                    effective_allowed_tools TEXT NOT NULL,
                    tool_calls TEXT NOT NULL,
                    outcome TEXT NOT NULL,
                    status TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );

                INSERT INTO agent_sessions(
                    session_id, task_id, identity_id, identity_name, identity_role, identity_block, skill_names, composed_context_hash, effective_allowed_tools,
                    tool_calls, outcome, status, created_at, updated_at
                )
                SELECT
                    session_id, task_id, NULL, NULL, NULL, NULL, skill_names, composed_context_hash, effective_allowed_tools,
                    tool_calls, outcome, status, created_at, updated_at
                FROM agent_sessions_legacy;

                DROP TABLE agent_sessions_legacy;
            "#,
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    }

    add_column_if_missing(
        conn,
        "ALTER TABLE agent_sessions ADD COLUMN identity_id TEXT",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE agent_sessions ADD COLUMN identity_name TEXT",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE agent_sessions ADD COLUMN identity_role TEXT",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE agent_sessions ADD COLUMN identity_block TEXT",
    )?;

    Ok(())
}

fn add_column_if_missing(conn: &Connection, sql: &str) -> Result<(), OrbitError> {
    match conn.execute(sql, []) {
        Ok(_) => Ok(()),
        Err(e) if e.to_string().contains("duplicate column name") => Ok(()),
        Err(e) => Err(OrbitError::Store(e.to_string())),
    }
}

fn ensure_tools_schema(conn: &Connection) -> Result<(), OrbitError> {
    add_column_if_missing(
        conn,
        "ALTER TABLE tools ADD COLUMN parameters_json TEXT NOT NULL DEFAULT '[]'",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE tools ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE tools ADD COLUMN builtin INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE tools ADD COLUMN created_at TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE tools ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''",
    )?;

    if table_has_column(conn, "tools", "is_enabled")? {
        conn.execute(
            r#"
                UPDATE tools
                SET enabled = CASE
                    WHEN lower(CAST(is_enabled AS TEXT)) IN ('0', 'false', 'f', 'no') THEN 0
                    ELSE 1
                END
            "#,
            [],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    }

    if table_has_column(conn, "tools", "is_builtin")? {
        conn.execute(
            r#"
                UPDATE tools
                SET builtin = CASE
                    WHEN lower(CAST(is_builtin AS TEXT)) IN ('1', 'true', 't', 'yes') THEN 1
                    ELSE 0
                END
            "#,
            [],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    }

    conn.execute(
        "UPDATE tools SET parameters_json = '[]' WHERE parameters_json = ''",
        [],
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;
    conn.execute(
        "UPDATE tools SET created_at = datetime('now') WHERE created_at = ''",
        [],
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;
    conn.execute(
        "UPDATE tools SET updated_at = datetime('now') WHERE updated_at = ''",
        [],
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    Ok(())
}

fn ensure_audit_events_schema(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
            CREATE TABLE IF NOT EXISTS audit_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                execution_id TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                command TEXT NOT NULL,
                subcommand TEXT,
                tool_name TEXT,
                target_type TEXT,
                target_id TEXT,
                role TEXT NOT NULL,
                status TEXT NOT NULL,
                exit_code INTEGER NOT NULL,
                duration_ms INTEGER NOT NULL,
                working_directory TEXT NOT NULL,
                arguments_json TEXT,
                stdout_truncated TEXT,
                stderr_truncated TEXT,
                error_message TEXT,
                host TEXT,
                pid INTEGER NOT NULL,
                session_id TEXT,
                task_id TEXT,
                job_run_id TEXT,
                activity_id TEXT,
                step_index INTEGER
            );

            CREATE INDEX IF NOT EXISTS idx_audit_events_timestamp
            ON audit_events(timestamp);

            CREATE INDEX IF NOT EXISTS idx_audit_events_tool_name
            ON audit_events(tool_name);

            CREATE INDEX IF NOT EXISTS idx_audit_events_status
            ON audit_events(status);

            CREATE INDEX IF NOT EXISTS idx_audit_events_role
            ON audit_events(role);

            CREATE INDEX IF NOT EXISTS idx_audit_events_target
            ON audit_events(target_type, target_id);

            CREATE UNIQUE INDEX IF NOT EXISTS idx_audit_events_execution_id
            ON audit_events(execution_id);
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    add_column_if_missing(conn, "ALTER TABLE audit_events ADD COLUMN task_id TEXT")?;
    add_column_if_missing(conn, "ALTER TABLE audit_events ADD COLUMN job_run_id TEXT")?;
    add_column_if_missing(conn, "ALTER TABLE audit_events ADD COLUMN activity_id TEXT")?;
    add_column_if_missing(
        conn,
        "ALTER TABLE audit_events ADD COLUMN step_index INTEGER",
    )?;

    conn.execute_batch(
        r#"
            CREATE INDEX IF NOT EXISTS idx_audit_events_task_id
            ON audit_events(task_id);

            CREATE INDEX IF NOT EXISTS idx_audit_events_job_run_id
            ON audit_events(job_run_id);
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    Ok(())
}

fn ensure_invocation_schema(conn: &Connection) -> Result<(), OrbitError> {
    add_column_if_missing(conn, "ALTER TABLE invocations ADD COLUMN slot TEXT")?;
    conn.execute_batch(
        r#"
            CREATE INDEX IF NOT EXISTS idx_invocations_job_run_id
            ON invocations(job_run_id);

            CREATE INDEX IF NOT EXISTS idx_invocations_activity_id
            ON invocations(activity_id);

            CREATE INDEX IF NOT EXISTS idx_invocation_tasks_task_id
            ON invocation_tasks(task_id);

            CREATE INDEX IF NOT EXISTS idx_tool_calls_tool_name
            ON tool_calls(tool_name);
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))
}

fn ensure_v2_state_consolidation_schema(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
            CREATE TABLE IF NOT EXISTS v2_audit_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                workspace_id TEXT NOT NULL,
                event_id TEXT NOT NULL,
                source TEXT NOT NULL,
                schema_version INTEGER NOT NULL,
                event_type TEXT NOT NULL,
                ts TEXT NOT NULL,
                run_id TEXT NOT NULL,
                agent_identity TEXT NOT NULL,
                parent_event_id TEXT,
                workspace_path TEXT,
                payload_json TEXT NOT NULL,
                UNIQUE(workspace_id, event_id)
            );

            CREATE INDEX IF NOT EXISTS idx_v2_audit_events_ws_ts
            ON v2_audit_events(workspace_id, ts);

            CREATE INDEX IF NOT EXISTS idx_v2_audit_events_ws_run
            ON v2_audit_events(workspace_id, run_id, ts);

            CREATE INDEX IF NOT EXISTS idx_v2_audit_events_ws_event_type
            ON v2_audit_events(workspace_id, event_type);

            CREATE TABLE IF NOT EXISTS job_runs (
                run_id TEXT NOT NULL,
                workspace_id TEXT NOT NULL,
                job_id TEXT NOT NULL,
                attempt INTEGER NOT NULL,
                state TEXT NOT NULL,
                scheduled_at TEXT NOT NULL,
                started_at TEXT,
                finished_at TEXT,
                duration_ms INTEGER,
                created_at TEXT NOT NULL,
                pid INTEGER,
                pid_start_time TEXT,
                input_json TEXT,
                retry_source_run_id TEXT,
                knowledge_metrics_json TEXT,
                resolved_crew TEXT,
                crew_model TEXT,
                planner_model TEXT,
                implementer_model TEXT,
                reviewer_model TEXT,
                pipeline_state_json TEXT,
                archived_at TEXT,
                PRIMARY KEY(workspace_id, run_id)
            );

            CREATE INDEX IF NOT EXISTS idx_job_runs_ws_job_sched
            ON job_runs(workspace_id, job_id, scheduled_at DESC);

            CREATE INDEX IF NOT EXISTS idx_job_runs_ws_state
            ON job_runs(workspace_id, state);

            CREATE TABLE IF NOT EXISTS job_run_steps (
                workspace_id TEXT NOT NULL,
                run_id TEXT NOT NULL,
                step_index INTEGER NOT NULL,
                target_type TEXT NOT NULL,
                target_id TEXT NOT NULL,
                state TEXT NOT NULL,
                started_at TEXT,
                finished_at TEXT,
                duration_ms INTEGER,
                exit_code INTEGER,
                error_code TEXT,
                error_message TEXT,
                agent_response_json TEXT,
                PRIMARY KEY(workspace_id, run_id, step_index),
                FOREIGN KEY(workspace_id, run_id)
                    REFERENCES job_runs(workspace_id, run_id)
                    ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS session_learning_state (
                workspace_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                learning_injection_state_json TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY(workspace_id, session_id)
            );

            CREATE INDEX IF NOT EXISTS idx_session_learning_state_ws
            ON session_learning_state(workspace_id, updated_at);

            CREATE TABLE IF NOT EXISTS schema_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))
}

fn apply_flat_crew_model(conn: &Connection) -> Result<(), OrbitError> {
    // ADR-0213: keep the legacy role columns nullable for existing databases;
    // new reads fall back to implementer_model when crew_model is not populated.
    add_column_if_missing(conn, "ALTER TABLE job_runs ADD COLUMN crew_model TEXT")
}

fn apply_job_run_archive_stage(conn: &Connection) -> Result<(), OrbitError> {
    add_column_if_missing(conn, "ALTER TABLE job_runs ADD COLUMN archived_at TEXT")
}

/// v5 `host_registry_core` migration (ORB-10255): durable machine identity,
/// lifecycle state, and immutable historical names in the hub-global store.
///
/// This migration is strictly additive. Cross-table triggers backstop the
/// typed API's preflight so a current or tombstoned `host_id` can never exist
/// for two machines, even when the database is modified outside that API.
fn apply_host_registry_core(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
            CREATE TABLE IF NOT EXISTS hosts (
                machine_id    TEXT PRIMARY KEY,
                host_id       TEXT NOT NULL UNIQUE,
                labels_json   TEXT NOT NULL DEFAULT '[]',
                status        TEXT NOT NULL CHECK (status IN ('active', 'retired')),
                registered_at TEXT NOT NULL,
                updated_at    TEXT NOT NULL,
                retired_at    TEXT,
                last_seen_at  TEXT,
                CHECK (length(machine_id) > 0),
                CHECK (length(host_id) > 0),
                CHECK (json_valid(labels_json) AND json_type(labels_json) = 'array'),
                CHECK (
                    (status = 'active' AND retired_at IS NULL)
                    OR (status = 'retired' AND retired_at IS NOT NULL)
                )
            );

            CREATE TABLE IF NOT EXISTS host_aliases (
                host_id    TEXT PRIMARY KEY,
                machine_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                warning    TEXT NOT NULL,
                CHECK (length(host_id) > 0),
                CHECK (length(warning) > 0),
                FOREIGN KEY(machine_id) REFERENCES hosts(machine_id)
                    ON UPDATE RESTRICT ON DELETE RESTRICT
            );

            CREATE INDEX IF NOT EXISTS idx_hosts_status_host_id
                ON hosts(status, host_id);
            CREATE INDEX IF NOT EXISTS idx_host_aliases_machine_id
                ON host_aliases(machine_id, created_at);

            CREATE TRIGGER IF NOT EXISTS hosts_host_id_not_alias_insert
            BEFORE INSERT ON hosts
            WHEN EXISTS (
                SELECT 1 FROM host_aliases WHERE host_id = NEW.host_id
            )
            BEGIN
                SELECT RAISE(ABORT, 'host_id is reserved by a permanent alias');
            END;

            CREATE TRIGGER IF NOT EXISTS hosts_host_id_not_alias_update
            BEFORE UPDATE OF host_id ON hosts
            WHEN EXISTS (
                SELECT 1 FROM host_aliases WHERE host_id = NEW.host_id
            )
            BEGIN
                SELECT RAISE(ABORT, 'host_id is reserved by a permanent alias');
            END;

            CREATE TRIGGER IF NOT EXISTS host_alias_not_current_name_insert
            BEFORE INSERT ON host_aliases
            WHEN EXISTS (
                SELECT 1 FROM hosts WHERE host_id = NEW.host_id
            )
            BEGIN
                SELECT RAISE(ABORT, 'host alias conflicts with a current host_id');
            END;

            CREATE TRIGGER IF NOT EXISTS host_aliases_immutable_update
            BEFORE UPDATE ON host_aliases
            BEGIN
                SELECT RAISE(ABORT, 'host aliases are immutable');
            END;

            CREATE TRIGGER IF NOT EXISTS host_aliases_immutable_delete
            BEFORE DELETE ON host_aliases
            BEGIN
                SELECT RAISE(ABORT, 'host aliases are permanent');
            END;
        "#,
    )
    .map_err(|error| OrbitError::Store(error.to_string()))
}

fn ensure_task_reservations_schema(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
            CREATE TABLE IF NOT EXISTS task_reservations (
                reservation_id TEXT PRIMARY KEY,
                workspace_orbit_dir TEXT NOT NULL,
                task_ids_json TEXT NOT NULL,
                files_json TEXT NOT NULL,
                actor TEXT NOT NULL,
                created_at TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                released_at TEXT,
                owner_run_id TEXT,
                owner_metadata_json TEXT,
                release_reason TEXT,
                release_metadata_json TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_task_reservations_workspace_expires
            ON task_reservations(workspace_orbit_dir, expires_at);

            CREATE INDEX IF NOT EXISTS idx_task_reservations_workspace_release
            ON task_reservations(workspace_orbit_dir, released_at);
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    add_column_if_missing(
        conn,
        "ALTER TABLE task_reservations ADD COLUMN workspace_id TEXT",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE task_reservations ADD COLUMN owner_run_id TEXT",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE task_reservations ADD COLUMN owner_metadata_json TEXT",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE task_reservations ADD COLUMN release_reason TEXT",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE task_reservations ADD COLUMN release_metadata_json TEXT",
    )?;

    conn.execute_batch(
        r#"
            CREATE INDEX IF NOT EXISTS idx_task_reservations_workspace_owner_release
            ON task_reservations(workspace_orbit_dir, owner_run_id, released_at);

            CREATE INDEX IF NOT EXISTS idx_task_reservations_workspace_id_release
            ON task_reservations(workspace_id, released_at);

            CREATE INDEX IF NOT EXISTS idx_task_reservations_workspace_id_expires
            ON task_reservations(workspace_id, expires_at);
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    Ok(())
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool, OrbitError> {
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = ?1",
            [table],
            |row| row.get(0),
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    Ok(exists > 0)
}

fn table_has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, OrbitError> {
    let pragma = format!("PRAGMA table_info({table})");
    let mut stmt = conn
        .prepare(&pragma)
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| OrbitError::Store(e.to_string()))?;

    for name in rows {
        let name = name.map_err(|e| OrbitError::Store(e.to_string()))?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn table_has_foreign_key_to(
    conn: &Connection,
    table: &str,
    referenced_table: &str,
) -> Result<bool, OrbitError> {
    let pragma = format!("PRAGMA foreign_key_list({table})");
    let mut stmt = conn
        .prepare(&pragma)
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(2))
        .map_err(|e| OrbitError::Store(e.to_string()))?;

    for name in rows {
        let name = name.map_err(|e| OrbitError::Store(e.to_string()))?;
        if name == referenced_table {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests;
