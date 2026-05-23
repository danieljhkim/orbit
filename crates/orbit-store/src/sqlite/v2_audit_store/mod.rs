use chrono::{DateTime, Utc};
use orbit_common::types::OrbitError;

use crate::{Store, parse_timestamp};

#[derive(Debug, Clone)]
pub struct V2AuditEventInsertParams {
    pub workspace_id: String,
    pub event_id: String,
    pub source: String,
    pub schema_version: u32,
    pub event_type: String,
    pub ts: DateTime<Utc>,
    pub run_id: String,
    pub agent_identity: String,
    pub parent_event_id: Option<String>,
    pub workspace_path: Option<String>,
    pub payload_json: String,
}

#[derive(Debug, Clone, Default)]
pub struct V2AuditEventFilter {
    pub workspace_id: String,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub run_id: Option<String>,
    pub event_type: Option<String>,
    pub source: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2AuditEventRow {
    pub id: i64,
    pub workspace_id: String,
    pub event_id: String,
    pub source: String,
    pub schema_version: u32,
    pub event_type: String,
    pub ts: DateTime<Utc>,
    pub run_id: String,
    pub agent_identity: String,
    pub parent_event_id: Option<String>,
    pub workspace_path: Option<String>,
    pub payload_json: String,
}

impl Store {
    pub fn insert_v2_audit_event(
        &self,
        params: &V2AuditEventInsertParams,
    ) -> Result<(), OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        conn.execute(
            r#"INSERT OR IGNORE INTO v2_audit_events(
                workspace_id, event_id, source, schema_version, event_type, ts,
                run_id, agent_identity, parent_event_id, workspace_path, payload_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
            rusqlite::params![
                params.workspace_id,
                params.event_id,
                params.source,
                i64::from(params.schema_version),
                params.event_type,
                params.ts.to_rfc3339(),
                params.run_id,
                params.agent_identity,
                params.parent_event_id,
                params.workspace_path,
                params.payload_json,
            ],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(())
    }

    pub fn list_v2_audit_events(
        &self,
        filter: &V2AuditEventFilter,
    ) -> Result<Vec<V2AuditEventRow>, OrbitError> {
        let (where_clause, params) = v2_filter_sql(filter);
        let limit = filter.limit.unwrap_or(1000);
        let offset = filter.offset.unwrap_or(0);
        let sql = format!(
            "SELECT id, workspace_id, event_id, source, schema_version, event_type, ts, \
             run_id, agent_identity, parent_event_id, workspace_path, payload_json \
             FROM v2_audit_events {where_clause} ORDER BY ts DESC, id DESC \
             LIMIT ?{} OFFSET ?{}",
            params.len() + 1,
            params.len() + 2
        );
        let mut params = params;
        params.push(Box::new(limit as i64));
        params.push(Box::new(offset as i64));
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|b| b.as_ref()).collect();

        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let rows = stmt
            .query_map(param_refs.as_slice(), row_to_v2_audit_event)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        collect_rows(rows)
    }

    pub fn count_v2_audit_events(&self, filter: &V2AuditEventFilter) -> Result<i64, OrbitError> {
        let (where_clause, params) = v2_filter_sql(filter);
        let sql = format!("SELECT COUNT(*) FROM v2_audit_events {where_clause}");
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|b| b.as_ref()).collect();
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        conn.query_row(&sql, param_refs.as_slice(), |row| row.get(0))
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn prune_v2_audit_events_older_than(
        &self,
        workspace_id: &str,
        ts: &DateTime<Utc>,
    ) -> Result<usize, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        conn.execute(
            "DELETE FROM v2_audit_events WHERE workspace_id = ?1 AND ts < ?2",
            rusqlite::params![workspace_id, ts.to_rfc3339()],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))
    }
}

fn v2_filter_sql(filter: &V2AuditEventFilter) -> (String, Vec<Box<dyn rusqlite::types::ToSql>>) {
    let mut conditions = vec!["workspace_id = ?1".to_string()];
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(filter.workspace_id.clone())];
    if let Some(since) = filter.since {
        conditions.push(format!("ts >= ?{}", params.len() + 1));
        params.push(Box::new(since.to_rfc3339()));
    }
    if let Some(until) = filter.until {
        conditions.push(format!("ts <= ?{}", params.len() + 1));
        params.push(Box::new(until.to_rfc3339()));
    }
    if let Some(run_id) = &filter.run_id {
        conditions.push(format!("run_id = ?{}", params.len() + 1));
        params.push(Box::new(run_id.clone()));
    }
    if let Some(event_type) = &filter.event_type {
        conditions.push(format!("event_type = ?{}", params.len() + 1));
        params.push(Box::new(event_type.clone()));
    }
    if let Some(source) = &filter.source {
        conditions.push(format!("source = ?{}", params.len() + 1));
        params.push(Box::new(source.clone()));
    }
    (format!("WHERE {}", conditions.join(" AND ")), params)
}

fn row_to_v2_audit_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<V2AuditEventRow> {
    let ts_raw: String = row.get(6)?;
    let schema_version: i64 = row.get(4)?;
    Ok(V2AuditEventRow {
        id: row.get(0)?,
        workspace_id: row.get(1)?,
        event_id: row.get(2)?,
        source: row.get(3)?,
        schema_version: schema_version as u32,
        event_type: row.get(5)?,
        ts: parse_timestamp(&ts_raw)?,
        run_id: row.get(7)?,
        agent_identity: row.get(8)?,
        parent_event_id: row.get(9)?,
        workspace_path: row.get(10)?,
        payload_json: row.get(11)?,
    })
}

fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>, OrbitError> {
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| OrbitError::Store(e.to_string()))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    #[test]
    fn insert_list_and_count_are_workspace_scoped() {
        let store = Store::open_in_memory().expect("store");
        let ts = Utc::now();
        for workspace_id in ["ws_a", "ws_b"] {
            store
                .insert_v2_audit_event(&V2AuditEventInsertParams {
                    workspace_id: workspace_id.to_string(),
                    event_id: format!("evt-{workspace_id}"),
                    source: "v2_envelope".to_string(),
                    schema_version: 1,
                    event_type: "tool.denied".to_string(),
                    ts,
                    run_id: "run-1".to_string(),
                    agent_identity: "codex".to_string(),
                    parent_event_id: None,
                    workspace_path: None,
                    payload_json: "{}".to_string(),
                })
                .expect("insert");
        }

        let filter = V2AuditEventFilter {
            workspace_id: "ws_a".to_string(),
            event_type: Some("tool.denied".to_string()),
            ..Default::default()
        };
        let rows = store.list_v2_audit_events(&filter).expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].event_id, "evt-ws_a");
        assert_eq!(store.count_v2_audit_events(&filter).expect("count"), 1);
    }
}
