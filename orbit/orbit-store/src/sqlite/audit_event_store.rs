use chrono::{DateTime, Utc};
use orbit_types::{AuditEvent, AuditEventStatus, OrbitError};
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
}

#[derive(Debug, Clone, Default)]
pub struct AuditEventFilter {
    pub since: Option<DateTime<Utc>>,
    pub tool_name: Option<String>,
    pub status: Option<AuditEventStatus>,
    pub role: Option<String>,
    pub limit: usize,
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
                host, pid, session_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)"#,
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
             error_message, host, pid, session_id \
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
                 error_message, host, pid, session_id \
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
