//! Audit-event SQL queries backing the `orbit audit list` CLI.
//!
//! L-0009: callers should reach audit data via `orbit audit list --json` —
//! `<workspace>/.orbit/orbit.db` (and -shm/-wal siblings) is an abandoned
//! leftover from pre-two-root binaries, not a mirror of the canonical global
//! `~/.orbit/orbit.db`. The CLI and runtime always use the global store.

use chrono::{DateTime, Utc};
use orbit_common::types::{AuditEvent, AuditEventStatus, OrbitError};
use rusqlite::params;

use crate::{Store, now_string, parse_timestamp};

#[derive(Debug, Clone)]
pub struct AuditEventInsertParams {
    pub execution_id: String,
    pub command: String,
    pub subcommand: Option<String>,
    pub tool_name: Option<String>,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub role: String,
    pub status: AuditEventStatus,
    pub exit_code: i32,
    pub duration_ms: i64,
    pub working_directory: String,
    pub arguments_json: Option<String>,
    pub stdout_truncated: Option<String>,
    pub stderr_truncated: Option<String>,
    pub error_message: Option<String>,
    pub host: Option<String>,
    pub pid: u32,
    pub session_id: Option<String>,
    pub task_id: Option<String>,
    pub job_run_id: Option<String>,
    pub activity_id: Option<String>,
    pub step_index: Option<i64>,
    pub backend: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AuditEventFilter {
    pub since: Option<DateTime<Utc>>,
    pub tool_name: Option<String>,
    pub target_type: Option<String>,
    pub status: Option<AuditEventStatus>,
    pub role: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditToolCallCountsByRole {
    pub role: String,
    pub total: u64,
    pub failed: u64,
}

/// Per-(surface, role) aggregate of `orbit.<surface>.*` tool calls. `surface`
/// is the segment between the leading `orbit.` namespace prefix and the next
/// dot — e.g. `orbit.graph.search` → `graph`, `orbit.task.update` → `task`.
/// Non-`orbit.*` tool names are excluded by the SQL filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditToolCallCountsBySurfaceAndRole {
    pub surface: String,
    pub role: String,
    pub total: u64,
    pub failed: u64,
}

/// One (role, tool_name) pair with its call count. Used to surface the
/// "most-called tools" leaderboard on the public Metrics page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditTopToolCall {
    pub role: String,
    pub tool_name: String,
    pub total: u64,
}

/// Per-tool aggregate of audit events across the full event population
/// (not just `command='tool'`). NULL `tool_name` rows are folded into a
/// synthetic `"unknown"` bucket so the dashboard never has to render a
/// blank tool name. Backs the Failures / Duration / Failure-rate cards
/// in the audit-summary side panel.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditToolAggregate {
    pub tool_name: String,
    pub total: i64,
    pub failures: i64,
    pub mcp_total: i64,
    pub cli_total: i64,
    pub mcp_failures: i64,
    pub cli_failures: i64,
    pub avg_duration_ms: f64,
}

/// Per-role aggregate of audit events with MCP/CLI surface split. Backs
/// the Role split and MCP-vs-CLI cards in the audit-summary side panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRoleAggregate {
    pub role: String,
    pub total: i64,
    pub mcp: i64,
    pub cli: i64,
}

impl Store {
    pub fn insert_audit_event_record(
        &self,
        params: &AuditEventInsertParams,
    ) -> Result<(), OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        conn.execute(
            r#"INSERT INTO audit_events(
                execution_id, timestamp, command, subcommand, tool_name,
                target_type, target_id, role, status, exit_code,
                duration_ms, working_directory, arguments_json,
                stdout_truncated, stderr_truncated, error_message,
                host, pid, session_id, task_id, job_run_id, activity_id,
                step_index, backend
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)"#,
            rusqlite::params![
                params.execution_id,
                now_string(),
                params.command,
                params.subcommand,
                params.tool_name,
                params.target_type,
                params.target_id,
                params.role,
                params.status.to_string(),
                params.exit_code,
                params.duration_ms,
                params.working_directory,
                params.arguments_json,
                params.stdout_truncated,
                params.stderr_truncated,
                params.error_message,
                params.host,
                params.pid,
                params.session_id,
                params.task_id,
                params.job_run_id,
                params.activity_id,
                params.step_index,
                params.backend,
            ],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(())
    }

    pub fn list_audit_events(
        &self,
        filter: &AuditEventFilter,
    ) -> Result<Vec<AuditEvent>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let mut conditions = Vec::new();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref since) = filter.since {
            conditions.push(format!("timestamp >= ?{}", param_values.len() + 1));
            param_values.push(Box::new(since.to_rfc3339()));
        }
        if let Some(ref tool) = filter.tool_name {
            conditions.push(format!("tool_name = ?{}", param_values.len() + 1));
            param_values.push(Box::new(tool.clone()));
        }
        if let Some(ref target_type) = filter.target_type {
            conditions.push(format!("target_type = ?{}", param_values.len() + 1));
            param_values.push(Box::new(target_type.clone()));
        }
        if let Some(ref status) = filter.status {
            conditions.push(format!("status = ?{}", param_values.len() + 1));
            param_values.push(Box::new(status.to_string()));
        }
        if let Some(ref role) = filter.role {
            conditions.push(format!("role = ?{}", param_values.len() + 1));
            param_values.push(Box::new(role.clone()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let limit = if filter.limit == 0 {
            1000
        } else {
            filter.limit
        };

        let sql = format!(
            "SELECT id, execution_id, timestamp, command, subcommand, tool_name, \
             target_type, target_id, role, status, exit_code, duration_ms, \
             working_directory, arguments_json, stdout_truncated, stderr_truncated, \
             error_message, host, pid, session_id, task_id, job_run_id, activity_id, \
             step_index, backend \
             FROM audit_events {where_clause} ORDER BY id DESC LIMIT ?{}",
            param_values.len() + 1
        );

        param_values.push(Box::new(limit as i64));

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|b| b.as_ref()).collect();

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                let ts_raw: String = row.get(2)?;
                let status_raw: String = row.get(9)?;

                let timestamp = parse_timestamp(&ts_raw)?;
                let status: AuditEventStatus = status_raw.parse().map_err(|e: String| {
                    rusqlite::Error::FromSqlConversionFailure(
                        status_raw.len(),
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
                    )
                })?;

                Ok(AuditEvent {
                    id: row.get(0)?,
                    execution_id: row.get(1)?,
                    timestamp,
                    command: row.get(3)?,
                    subcommand: row.get(4)?,
                    tool_name: row.get(5)?,
                    target_type: row.get(6)?,
                    target_id: row.get(7)?,
                    role: row.get(8)?,
                    status,
                    exit_code: row.get(10)?,
                    duration_ms: row.get(11)?,
                    working_directory: row.get(12)?,
                    arguments_json: row.get(13)?,
                    stdout_truncated: row.get(14)?,
                    stderr_truncated: row.get(15)?,
                    error_message: row.get(16)?,
                    host: row.get(17)?,
                    pid: row.get(18)?,
                    session_id: row.get(19)?,
                    task_id: row.get(20)?,
                    job_run_id: row.get(21)?,
                    activity_id: row.get(22)?,
                    step_index: row.get(23)?,
                    backend: row.get(24)?,
                })
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn get_audit_event(&self, id: i64) -> Result<Option<AuditEvent>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, execution_id, timestamp, command, subcommand, tool_name, \
                 target_type, target_id, role, status, exit_code, duration_ms, \
                 working_directory, arguments_json, stdout_truncated, stderr_truncated, \
                 error_message, host, pid, session_id, task_id, job_run_id, activity_id, \
                 step_index, backend \
                 FROM audit_events WHERE id = ?1",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let result = stmt
            .query_row(params![id], |row| {
                let ts_raw: String = row.get(2)?;
                let status_raw: String = row.get(9)?;

                let timestamp = parse_timestamp(&ts_raw)?;
                let status: AuditEventStatus = status_raw.parse().map_err(|e: String| {
                    rusqlite::Error::FromSqlConversionFailure(
                        status_raw.len(),
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
                    )
                })?;

                Ok(AuditEvent {
                    id: row.get(0)?,
                    execution_id: row.get(1)?,
                    timestamp,
                    command: row.get(3)?,
                    subcommand: row.get(4)?,
                    tool_name: row.get(5)?,
                    target_type: row.get(6)?,
                    target_id: row.get(7)?,
                    role: row.get(8)?,
                    status,
                    exit_code: row.get(10)?,
                    duration_ms: row.get(11)?,
                    working_directory: row.get(12)?,
                    arguments_json: row.get(13)?,
                    stdout_truncated: row.get(14)?,
                    stderr_truncated: row.get(15)?,
                    error_message: row.get(16)?,
                    host: row.get(17)?,
                    pid: row.get(18)?,
                    session_id: row.get(19)?,
                    task_id: row.get(20)?,
                    job_run_id: row.get(21)?,
                    activity_id: row.get(22)?,
                    step_index: row.get(23)?,
                    backend: row.get(24)?,
                })
            })
            .optional()
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(result)
    }

    pub fn get_audit_event_stats(
        &self,
        since: Option<&DateTime<Utc>>,
        tool: Option<&str>,
    ) -> Result<(i64, i64, i64, i64, f64, i64), OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let mut conditions = Vec::new();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(since) = since {
            conditions.push(format!("timestamp >= ?{}", param_values.len() + 1));
            param_values.push(Box::new(since.to_rfc3339()));
        }
        if let Some(tool) = tool {
            conditions.push(format!("tool_name = ?{}", param_values.len() + 1));
            param_values.push(Box::new(tool.to_string()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT \
             COUNT(*), \
             COALESCE(SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END), 0), \
             COALESCE(SUM(CASE WHEN status = 'failure' THEN 1 ELSE 0 END), 0), \
             COALESCE(SUM(CASE WHEN status = 'denied' THEN 1 ELSE 0 END), 0), \
             COALESCE(AVG(duration_ms), 0.0), \
             COALESCE(MAX(duration_ms), 0) \
             FROM audit_events {where_clause}"
        );

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|b| b.as_ref()).collect();

        conn.query_row(&sql, param_refs.as_slice(), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn get_audit_event_durations(
        &self,
        since: Option<&DateTime<Utc>>,
        tool: Option<&str>,
    ) -> Result<Vec<i64>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let mut conditions = Vec::new();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(since) = since {
            conditions.push(format!("timestamp >= ?{}", param_values.len() + 1));
            param_values.push(Box::new(since.to_rfc3339()));
        }
        if let Some(tool) = tool {
            conditions.push(format!("tool_name = ?{}", param_values.len() + 1));
            param_values.push(Box::new(tool.to_string()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let sql =
            format!("SELECT duration_ms FROM audit_events {where_clause} ORDER BY duration_ms ASC");

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|b| b.as_ref()).collect();

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = stmt
            .query_map(param_refs.as_slice(), |row| row.get::<_, i64>(0))
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    /// Returns hourly buckets `(rfc3339_hour_start, count)` of audit events with
    /// `timestamp >= since`, ordered ascending by bucket. Bucket starts are
    /// truncated to `YYYY-MM-DDTHH:00:00Z`. Empty hours are NOT returned —
    /// callers must zero-fill missing hours when rendering a sparkline.
    pub fn get_audit_event_hourly_buckets(
        &self,
        since: &DateTime<Utc>,
    ) -> Result<Vec<(String, i64)>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let sql = "SELECT strftime('%Y-%m-%dT%H:00:00Z', timestamp) AS bucket, COUNT(*) \
                   FROM audit_events WHERE timestamp >= ?1 \
                   GROUP BY bucket ORDER BY bucket ASC";

        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = stmt
            .query_map(params![since.to_rfc3339()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    /// Returns `(role, denied_count)` for audit events with status='denied' and
    /// `timestamp >= since`, ordered desc by count. Used to join SQLite-level
    /// CLI denials onto the per-agent scoreboard.
    pub fn get_audit_denials_by_role(
        &self,
        since: Option<&DateTime<Utc>>,
    ) -> Result<Vec<(String, i64)>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let sql = if since.is_some() {
            "SELECT role, COUNT(*) FROM audit_events \
             WHERE status = 'denied' AND timestamp >= ?1 \
             GROUP BY role ORDER BY COUNT(*) DESC"
        } else {
            "SELECT role, COUNT(*) FROM audit_events \
             WHERE status = 'denied' \
             GROUP BY role ORDER BY COUNT(*) DESC"
        };

        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = if let Some(s) = since {
            stmt.query_map(params![s.to_rfc3339()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
        } else {
            stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
        };

        rows.map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn get_audit_tool_call_counts_by_role(
        &self,
        since: Option<&DateTime<Utc>>,
    ) -> Result<Vec<AuditToolCallCountsByRole>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let sql = if since.is_some() {
            "SELECT role, COUNT(*), \
             COALESCE(SUM(CASE WHEN status != 'success' THEN 1 ELSE 0 END), 0) \
             FROM audit_events \
             WHERE command = 'tool' \
               AND subcommand IN ('run', 'run-mcp') \
               AND tool_name IS NOT NULL \
               AND timestamp >= ?1 \
             GROUP BY role ORDER BY COUNT(*) DESC, role ASC"
        } else {
            "SELECT role, COUNT(*), \
             COALESCE(SUM(CASE WHEN status != 'success' THEN 1 ELSE 0 END), 0) \
             FROM audit_events \
             WHERE command = 'tool' \
               AND subcommand IN ('run', 'run-mcp') \
               AND tool_name IS NOT NULL \
             GROUP BY role ORDER BY COUNT(*) DESC, role ASC"
        };

        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = if let Some(s) = since {
            stmt.query_map(params![s.to_rfc3339()], |row| {
                Ok(AuditToolCallCountsByRole {
                    role: row.get(0)?,
                    total: row.get::<_, i64>(1)? as u64,
                    failed: row.get::<_, i64>(2)? as u64,
                })
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
        } else {
            stmt.query_map([], |row| {
                Ok(AuditToolCallCountsByRole {
                    role: row.get(0)?,
                    total: row.get::<_, i64>(1)? as u64,
                    failed: row.get::<_, i64>(2)? as u64,
                })
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
        };

        rows.map_err(|e| OrbitError::Store(e.to_string()))
    }

    /// Per-(surface, role) tool call counts where `tool_name` matches
    /// `orbit.<surface>.<verb>`. The surface segment is extracted with
    /// SQLite string functions so we don't need a regex extension.
    /// `failed` counts every non-`success` row (failure + denied) like
    /// [`Self::get_audit_tool_call_counts_by_role`].
    pub fn get_audit_tool_call_counts_by_surface_and_role(
        &self,
        since: Option<&DateTime<Utc>>,
    ) -> Result<Vec<AuditToolCallCountsBySurfaceAndRole>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        // SUBSTR(tool_name, 7) strips the literal "orbit." prefix; the
        // appended "." in the inner SUBSTR ensures INSTR finds a delimiter
        // even for names with no third segment (e.g. "orbit.task" → surface
        // "task"). The outer LIKE filter discards anything that does not
        // start with "orbit." entirely.
        let extract = "SUBSTR(tool_name, 7, INSTR(SUBSTR(tool_name, 7) || '.', '.') - 1)";
        let sql = if since.is_some() {
            format!(
                "SELECT {extract} AS surface, role, COUNT(*), \
                 COALESCE(SUM(CASE WHEN status != 'success' THEN 1 ELSE 0 END), 0) \
                 FROM audit_events \
                 WHERE command = 'tool' \
                   AND subcommand IN ('run', 'run-mcp') \
                   AND tool_name LIKE 'orbit.%' \
                   AND timestamp >= ?1 \
                 GROUP BY surface, role \
                 ORDER BY surface ASC, COUNT(*) DESC, role ASC"
            )
        } else {
            format!(
                "SELECT {extract} AS surface, role, COUNT(*), \
                 COALESCE(SUM(CASE WHEN status != 'success' THEN 1 ELSE 0 END), 0) \
                 FROM audit_events \
                 WHERE command = 'tool' \
                   AND subcommand IN ('run', 'run-mcp') \
                   AND tool_name LIKE 'orbit.%' \
                 GROUP BY surface, role \
                 ORDER BY surface ASC, COUNT(*) DESC, role ASC"
            )
        };

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = if let Some(s) = since {
            stmt.query_map(params![s.to_rfc3339()], |row| {
                Ok(AuditToolCallCountsBySurfaceAndRole {
                    surface: row.get(0)?,
                    role: row.get(1)?,
                    total: row.get::<_, i64>(2)? as u64,
                    failed: row.get::<_, i64>(3)? as u64,
                })
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
        } else {
            stmt.query_map([], |row| {
                Ok(AuditToolCallCountsBySurfaceAndRole {
                    surface: row.get(0)?,
                    role: row.get(1)?,
                    total: row.get::<_, i64>(2)? as u64,
                    failed: row.get::<_, i64>(3)? as u64,
                })
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
        };

        rows.map_err(|e| OrbitError::Store(e.to_string()))
    }

    /// Top (role, tool_name) pairs by call count across the audit log,
    /// limited to `orbit.*` tool names. The optional `since` filter, when
    /// supplied, scopes the query to events at-or-after that timestamp.
    /// `limit` caps the row count after sorting; `0` means no cap.
    ///
    /// Sort key: total DESC, then tool_name ASC, then role ASC for stable
    /// output across runs.
    pub fn get_audit_top_tool_calls(
        &self,
        since: Option<&DateTime<Utc>>,
        limit: usize,
    ) -> Result<Vec<AuditTopToolCall>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let base = "SELECT tool_name, role, COUNT(*) \
                    FROM audit_events \
                    WHERE command = 'tool' \
                      AND subcommand IN ('run', 'run-mcp') \
                      AND tool_name LIKE 'orbit.%'";
        let order = "GROUP BY tool_name, role \
                     ORDER BY COUNT(*) DESC, tool_name ASC, role ASC";
        let sql = match (since.is_some(), limit > 0) {
            (true, true) => format!("{base} AND timestamp >= ?1 {order} LIMIT ?2"),
            (true, false) => format!("{base} AND timestamp >= ?1 {order}"),
            (false, true) => format!("{base} {order} LIMIT ?1"),
            (false, false) => format!("{base} {order}"),
        };

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let map_row = |row: &rusqlite::Row<'_>| {
            Ok(AuditTopToolCall {
                tool_name: row.get(0)?,
                role: row.get(1)?,
                total: row.get::<_, i64>(2)? as u64,
            })
        };

        let rows = match (since, limit) {
            (Some(s), 0) => stmt
                .query_map(params![s.to_rfc3339()], map_row)
                .map_err(|e| OrbitError::Store(e.to_string()))?
                .collect::<Result<Vec<_>, _>>(),
            (Some(s), n) => stmt
                .query_map(params![s.to_rfc3339(), n as i64], map_row)
                .map_err(|e| OrbitError::Store(e.to_string()))?
                .collect::<Result<Vec<_>, _>>(),
            (None, 0) => stmt
                .query_map([], map_row)
                .map_err(|e| OrbitError::Store(e.to_string()))?
                .collect::<Result<Vec<_>, _>>(),
            (None, n) => stmt
                .query_map(params![n as i64], map_row)
                .map_err(|e| OrbitError::Store(e.to_string()))?
                .collect::<Result<Vec<_>, _>>(),
        };

        rows.map_err(|e| OrbitError::Store(e.to_string()))
    }

    /// Sorted `duration_ms` values for audit events with NULL `tool_name`
    /// at or after `since`. Mirror of [`Self::get_audit_event_durations`]
    /// for the synthetic `"unknown"` bucket that
    /// [`Self::get_audit_event_aggregates_by_tool`] surfaces — that aggregate
    /// folds NULL tool names into `"unknown"` for counts, and this method
    /// lets the caller compute the same bucket's percentiles.
    pub fn get_audit_event_durations_null_tool(
        &self,
        since: &DateTime<Utc>,
    ) -> Result<Vec<i64>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let sql = "SELECT duration_ms FROM audit_events \
                   WHERE tool_name IS NULL AND timestamp >= ?1 \
                   ORDER BY duration_ms ASC";

        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = stmt
            .query_map(params![since.to_rfc3339()], |row| row.get::<_, i64>(0))
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    /// Per-tool aggregate of audit events with `timestamp >= since`. Folds
    /// NULL `tool_name` into a synthetic `"unknown"` bucket so callers don't
    /// have to guard against missing values. The `mcp_*` / `cli_*` columns
    /// only count rows where `subcommand` is `'run-mcp'` or `'run'` respectively;
    /// other subcommands contribute to `total` and `failures` but not to the
    /// split.
    pub fn get_audit_event_aggregates_by_tool(
        &self,
        since: &DateTime<Utc>,
    ) -> Result<Vec<AuditToolAggregate>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let sql = "SELECT COALESCE(tool_name, 'unknown') AS tool, \
                   COUNT(*), \
                   COALESCE(SUM(CASE WHEN status = 'failure' THEN 1 ELSE 0 END), 0), \
                   COALESCE(SUM(CASE WHEN subcommand = 'run-mcp' THEN 1 ELSE 0 END), 0), \
                   COALESCE(SUM(CASE WHEN subcommand = 'run' THEN 1 ELSE 0 END), 0), \
                   COALESCE(SUM(CASE WHEN status = 'failure' AND subcommand = 'run-mcp' THEN 1 ELSE 0 END), 0), \
                   COALESCE(SUM(CASE WHEN status = 'failure' AND subcommand = 'run' THEN 1 ELSE 0 END), 0), \
                   COALESCE(AVG(duration_ms), 0.0) \
                   FROM audit_events WHERE timestamp >= ?1 GROUP BY tool";

        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = stmt
            .query_map(params![since.to_rfc3339()], |row| {
                Ok(AuditToolAggregate {
                    tool_name: row.get(0)?,
                    total: row.get(1)?,
                    failures: row.get(2)?,
                    mcp_total: row.get(3)?,
                    cli_total: row.get(4)?,
                    mcp_failures: row.get(5)?,
                    cli_failures: row.get(6)?,
                    avg_duration_ms: row.get(7)?,
                })
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    /// Per-role aggregate of audit events with `timestamp >= since`, including
    /// the MCP-vs-CLI surface split. Rows where `subcommand` is neither `'run'`
    /// nor `'run-mcp'` still count toward `total` but neither `mcp` nor `cli`.
    pub fn get_audit_event_aggregates_by_role(
        &self,
        since: &DateTime<Utc>,
    ) -> Result<Vec<AuditRoleAggregate>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let sql = "SELECT role, \
                   COUNT(*), \
                   COALESCE(SUM(CASE WHEN subcommand = 'run-mcp' THEN 1 ELSE 0 END), 0), \
                   COALESCE(SUM(CASE WHEN subcommand = 'run' THEN 1 ELSE 0 END), 0) \
                   FROM audit_events WHERE timestamp >= ?1 \
                   GROUP BY role ORDER BY COUNT(*) DESC, role ASC";

        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = stmt
            .query_map(params![since.to_rfc3339()], |row| {
                Ok(AuditRoleAggregate {
                    role: row.get(0)?,
                    total: row.get(1)?,
                    mcp: row.get(2)?,
                    cli: row.get(3)?,
                })
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn prune_audit_events(&self, older_than: &DateTime<Utc>) -> Result<usize, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let count = conn
            .execute(
                "DELETE FROM audit_events WHERE timestamp < ?1",
                params![older_than.to_rfc3339()],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(count)
    }
}

use rusqlite::OptionalExtension;

#[cfg(test)]
#[cfg(test)]
mod tests;
