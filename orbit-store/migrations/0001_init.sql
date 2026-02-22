-- v2.1 initial schema; runtime currently applies equivalent SQL via execute_batch.
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    instructions TEXT NOT NULL DEFAULT '',
    context_files TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL DEFAULT 'todo',
    priority TEXT NOT NULL DEFAULT 'medium',
    task_type TEXT NOT NULL DEFAULT 'task',
    owner TEXT NOT NULL DEFAULT '',
    parent_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS memos (
    id TEXT PRIMARY KEY,
    body TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    command TEXT NOT NULL,
    next_run_at TEXT NOT NULL,
    last_run_at TEXT,
    status TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS watches (
    id TEXT PRIMARY KEY,
    path TEXT NOT NULL,
    command TEXT NOT NULL,
    debounce_ms INTEGER NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS audits (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    event_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    message TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS locks (
    name TEXT PRIMARY KEY,
    owner TEXT NOT NULL,
    acquired_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tools (
    name TEXT PRIMARY KEY,
    path TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    enabled INTEGER NOT NULL DEFAULT 1,
    builtin INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS skills (
    schema_version INTEGER NOT NULL,
    name TEXT PRIMARY KEY,
    description TEXT,
    instructions TEXT NOT NULL,
    context_files TEXT NOT NULL DEFAULT '[]',
    allowed_tools TEXT NOT NULL DEFAULT '[]',
    role TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS task_skills (
    task_id TEXT NOT NULL,
    skill_name TEXT NOT NULL,
    attachment_order INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (task_id, skill_name),
    FOREIGN KEY(task_id) REFERENCES tasks(id) ON DELETE CASCADE,
    FOREIGN KEY(skill_name) REFERENCES skills(name) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS agent_sessions (
    session_id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    skill_names TEXT NOT NULL,
    composed_context_hash TEXT NOT NULL,
    effective_allowed_tools TEXT NOT NULL,
    tool_calls TEXT NOT NULL,
    outcome TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(task_id) REFERENCES tasks(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS entries (
    id TEXT PRIMARY KEY,
    entity_type TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    session_id TEXT,
    sequence_number INTEGER NOT NULL,
    entry_type TEXT NOT NULL,
    author_type TEXT NOT NULL,
    author_id TEXT NOT NULL,
    author_model TEXT,
    body TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_entries_entity_seq
ON entries(entity_type, entity_id, sequence_number);

CREATE INDEX IF NOT EXISTS idx_entries_entity
ON entries(entity_type, entity_id);

CREATE INDEX IF NOT EXISTS idx_entries_session
ON entries(session_id);

CREATE INDEX IF NOT EXISTS idx_entries_author
ON entries(author_type, author_id);
