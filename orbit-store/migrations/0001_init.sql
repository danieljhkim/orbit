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
    job_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    task_id TEXT NOT NULL,
    schedule_spec TEXT NOT NULL,
    timezone TEXT NOT NULL,
    state TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    paused_at TEXT,
    deleted_at TEXT,
    last_run_session_id TEXT,
    last_run_at TEXT,
    next_run_at TEXT,
    last_error TEXT
);

CREATE INDEX IF NOT EXISTS idx_jobs_state
ON jobs(state);

CREATE INDEX IF NOT EXISTS idx_jobs_task
ON jobs(task_id);

CREATE INDEX IF NOT EXISTS idx_jobs_next_run
ON jobs(state, next_run_at);

CREATE TABLE IF NOT EXISTS job_sessions (
    session_id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    trigger TEXT NOT NULL,
    trigger_time TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT,
    status TEXT NOT NULL,
    exit_code INTEGER,
    error TEXT,
    composed_context_hash TEXT,
    effective_allowlist_hash TEXT,
    created_by_role TEXT NOT NULL,
    created_at TEXT NOT NULL,
    cancel_requested_at TEXT,
    FOREIGN KEY(job_id) REFERENCES jobs(job_id)
);

CREATE INDEX IF NOT EXISTS idx_job_sessions_job
ON job_sessions(job_id, created_at);

CREATE INDEX IF NOT EXISTS idx_job_sessions_status
ON job_sessions(status);

CREATE UNIQUE INDEX IF NOT EXISTS uq_job_sessions_single_running
ON job_sessions(job_id)
WHERE status = 'running';

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
