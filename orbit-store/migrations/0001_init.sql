-- v2 initial schema; runtime applies equivalent SQL via execute_batch.
CREATE TABLE IF NOT EXISTS tools (
    name TEXT PRIMARY KEY,
    path TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
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
    session_id TEXT
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
