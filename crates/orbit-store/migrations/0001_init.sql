-- v2 initial schema; runtime applies equivalent SQL via execute_batch.
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
    session_id TEXT
);

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

CREATE TABLE IF NOT EXISTS task_tags (
    task_id TEXT NOT NULL,
    tag TEXT NOT NULL,
    PRIMARY KEY(task_id, tag)
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

CREATE INDEX IF NOT EXISTS idx_task_reservations_workspace_owner_release
ON task_reservations(workspace_orbit_dir, owner_run_id, released_at);

CREATE INDEX IF NOT EXISTS idx_task_reservations_workspace_expires
ON task_reservations(workspace_orbit_dir, expires_at);

CREATE INDEX IF NOT EXISTS idx_task_reservations_workspace_release
ON task_reservations(workspace_orbit_dir, released_at);

CREATE INDEX IF NOT EXISTS idx_task_tags_tag_task_id
ON task_tags(tag, task_id);

CREATE TABLE IF NOT EXISTS embeddings (
    source_kind TEXT NOT NULL,
    source_id TEXT NOT NULL,
    field TEXT NOT NULL,
    chunk_idx INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    model_id TEXT NOT NULL,
    dim INTEGER NOT NULL,
    embedding BLOB NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (source_kind, source_id, field, chunk_idx, model_id)
);

CREATE INDEX IF NOT EXISTS embeddings_by_source
ON embeddings(source_kind, source_id);

CREATE INDEX IF NOT EXISTS embeddings_by_model
ON embeddings(model_id);

CREATE VIRTUAL TABLE IF NOT EXISTS tasks_fts USING fts5(
    source_id UNINDEXED,
    field UNINDEXED,
    content,
    tokenize = 'porter unicode61 remove_diacritics 2'
);
