use std::collections::HashMap;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use rusqlite::{params, types::ToSql};

use orbit_common::types::{OrbitError, RoleSlot};

use crate::{Store, now_string};

use super::types::{
    InvocationInsertParams, InvocationQuery, InvocationRecord, InvocationToolCallRecord,
};

impl Store {
    pub fn insert_invocation_trace_record(
        &self,
        params: &InvocationInsertParams,
    ) -> Result<(), OrbitError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let tx = conn
            .transaction()
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        tx.execute(
            r#"INSERT INTO invocations(
                ts, job_run_id, activity_id, agent, model, slot, duration_ms,
                input_tokens, cache_read_tokens, cache_create_tokens,
                output_tokens, tool_call_count
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"#,
            params![
                now_string(),
                params.job_run_id,
                params.activity_id,
                params.agent,
                params.model,
                params.slot.map(|slot| slot.as_str().to_string()),
                params.trace.duration_ms as i64,
                params.trace.usage.input as i64,
                params.trace.usage.cache_read as i64,
                params.trace.usage.cache_create as i64,
                params.trace.usage.output as i64,
                params.trace.tool_calls.len() as i64,
            ],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;

        let invocation_id = tx.last_insert_rowid();
        insert_invocation_task_ids(&tx, invocation_id, &params.task_ids)?;
        insert_tool_calls(&tx, invocation_id, &params.trace.tool_calls)?;

        tx.commit().map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn list_invocation_records(
        &self,
        filter: &InvocationQuery,
    ) -> Result<Vec<InvocationRecord>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let (sql, params) = build_invocation_list_query(filter);
        let param_refs: Vec<&dyn ToSql> = params.iter().map(|value| value.as_ref()).collect();

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let rows = stmt
            .query_map(param_refs.as_slice(), map_invocation_record)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let mut records = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        drop(stmt);
        drop(conn);

        if records.is_empty() {
            return Ok(records);
        }

        hydrate_invocation_records(self, &mut records)?;
        Ok(records)
    }

    fn load_invocation_task_ids(
        &self,
        invocation_ids: &[i64],
    ) -> Result<HashMap<i64, Vec<String>>, OrbitError> {
        let conn = self.connection_handle()?;
        load_grouped_strings(
            &conn,
            invocation_ids,
            "SELECT invocation_id, task_id FROM invocation_tasks WHERE invocation_id IN ({placeholders}) ORDER BY invocation_id ASC, task_id ASC",
        )
    }

    fn load_invocation_tool_calls(
        &self,
        invocation_ids: &[i64],
    ) -> Result<HashMap<i64, Vec<InvocationToolCallRecord>>, OrbitError> {
        if invocation_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let conn = self.connection_handle()?;
        let placeholders = sql_placeholders(invocation_ids.len());
        let sql = format!(
            "SELECT invocation_id, seq, tool_name, result_bytes FROM tool_calls WHERE invocation_id IN ({placeholders}) ORDER BY invocation_id ASC, seq ASC"
        );
        let params: Vec<Box<dyn ToSql>> = invocation_ids
            .iter()
            .map(|id| Box::new(*id) as Box<dyn ToSql>)
            .collect();
        let param_refs: Vec<&dyn ToSql> = params.iter().map(|value| value.as_ref()).collect();

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok(InvocationToolCallRecord {
                    invocation_id: row.get(0)?,
                    seq: row.get::<_, i64>(1)? as u64,
                    tool_name: row.get(2)?,
                    result_bytes: row.get::<_, i64>(3)? as u64,
                })
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let mut grouped: HashMap<i64, Vec<InvocationToolCallRecord>> = HashMap::new();
        for row in rows {
            let call = row.map_err(|e| OrbitError::Store(e.to_string()))?;
            grouped.entry(call.invocation_id).or_default().push(call);
        }
        Ok(grouped)
    }
}

fn insert_invocation_task_ids(
    tx: &rusqlite::Transaction<'_>,
    invocation_id: i64,
    task_ids: &[String],
) -> Result<(), OrbitError> {
    for task_id in task_ids {
        tx.execute(
            "INSERT OR IGNORE INTO invocation_tasks(invocation_id, task_id) VALUES (?1, ?2)",
            params![invocation_id, task_id],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    }
    Ok(())
}

fn insert_tool_calls(
    tx: &rusqlite::Transaction<'_>,
    invocation_id: i64,
    tool_calls: &[orbit_common::types::ToolCallTrace],
) -> Result<(), OrbitError> {
    for tool_call in tool_calls {
        tx.execute(
            r#"INSERT INTO tool_calls(invocation_id, seq, tool_name, result_bytes)
               VALUES (?1, ?2, ?3, ?4)"#,
            params![
                invocation_id,
                tool_call.seq as i64,
                tool_call.tool_name,
                tool_call.result_bytes as i64,
            ],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    }
    Ok(())
}

fn build_invocation_list_query(filter: &InvocationQuery) -> (String, Vec<Box<dyn ToSql>>) {
    let mut query = InvocationListQuery::default();

    if let Some(since) = &filter.since {
        query.push_filter("i.ts >= ?", since.to_rfc3339());
    }
    if let Some(until) = &filter.until {
        query.push_filter("i.ts <= ?", until.to_rfc3339());
    }
    if let Some(job_run_id) = &filter.job_run_id {
        query.push_filter("i.job_run_id = ?", job_run_id.clone());
    }
    if let Some(activity_id) = &filter.activity_id {
        query.push_filter("i.activity_id = ?", activity_id.clone());
    }
    if let Some(task_id) = &filter.task_id {
        query.push_filter(
            "EXISTS (SELECT 1 FROM invocation_tasks it WHERE it.invocation_id = i.id AND it.task_id = ?)",
            task_id.clone(),
        );
    }
    if let Some(agent) = &filter.agent {
        query.push_filter("i.agent = ?", agent.clone());
    }
    if let Some(model) = &filter.model {
        query.push_filter("i.model = ?", model.clone());
    }
    if let Some(slot) = filter.slot {
        query.push_filter("i.slot = ?", slot.as_str().to_string());
    }
    if let Some(tool_name) = &filter.tool_name {
        query.push_filter(
            "EXISTS (SELECT 1 FROM tool_calls tc WHERE tc.invocation_id = i.id AND tc.tool_name = ?)",
            tool_name.clone(),
        );
    }

    let limit = if filter.limit == 0 { 100 } else { filter.limit };
    query.push_value(limit as i64);

    let sql = format!(
        "SELECT i.id, i.ts, i.job_run_id, i.activity_id, i.agent, i.model, i.slot, i.duration_ms, \
         i.input_tokens, i.cache_read_tokens, i.cache_create_tokens, i.output_tokens, \
         i.tool_call_count \
         FROM invocations i {} ORDER BY i.ts DESC, i.id DESC LIMIT ?{}",
        query.where_clause(),
        query.len()
    );

    (sql, query.params)
}

fn map_invocation_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<InvocationRecord> {
    let ts_raw: String = row.get(1)?;
    let slot_raw: Option<String> = row.get(6)?;
    let input_tokens = row.get::<_, i64>(8)? as u64;
    let output_tokens = row.get::<_, i64>(11)? as u64;

    Ok(InvocationRecord {
        id: row.get(0)?,
        ts: parse_rfc3339_timestamp(&ts_raw)?,
        job_run_id: row.get(2)?,
        activity_id: row.get(3)?,
        agent: row.get(4)?,
        model: row.get(5)?,
        slot: slot_raw
            .as_deref()
            .map(RoleSlot::from_str)
            .transpose()
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    6,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
        duration_ms: row.get::<_, i64>(7)? as u64,
        input_tokens,
        cache_read_tokens: row.get::<_, i64>(9)? as u64,
        cache_create_tokens: row.get::<_, i64>(10)? as u64,
        output_tokens,
        total_tokens: input_tokens.saturating_add(output_tokens),
        tool_call_count: row.get::<_, i64>(12)? as u64,
        task_ids: Vec::new(),
        tool_calls: Vec::new(),
    })
}

fn hydrate_invocation_records(
    store: &Store,
    records: &mut [InvocationRecord],
) -> Result<(), OrbitError> {
    let invocation_ids = records.iter().map(|record| record.id).collect::<Vec<_>>();
    let task_ids = store.load_invocation_task_ids(&invocation_ids)?;
    let tool_calls = store.load_invocation_tool_calls(&invocation_ids)?;
    let index_by_id = records
        .iter()
        .enumerate()
        .map(|(index, record)| (record.id, index))
        .collect::<HashMap<_, _>>();

    for (invocation_id, values) in task_ids {
        if let Some(index) = index_by_id.get(&invocation_id).copied() {
            records[index].task_ids = values;
        }
    }
    for (invocation_id, values) in tool_calls {
        if let Some(index) = index_by_id.get(&invocation_id).copied() {
            records[index].tool_calls = values;
        }
    }

    Ok(())
}

fn parse_rfc3339_timestamp(raw: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                raw.len(),
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
}

fn load_grouped_strings(
    conn: &rusqlite::Connection,
    invocation_ids: &[i64],
    sql_template: &str,
) -> Result<HashMap<i64, Vec<String>>, OrbitError> {
    if invocation_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let placeholders = sql_placeholders(invocation_ids.len());
    let sql = sql_template.replace("{placeholders}", &placeholders);
    let params: Vec<Box<dyn ToSql>> = invocation_ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn ToSql>)
        .collect();
    let param_refs: Vec<&dyn ToSql> = params.iter().map(|value| value.as_ref()).collect();

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    let rows = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| OrbitError::Store(e.to_string()))?;

    let mut grouped: HashMap<i64, Vec<String>> = HashMap::new();
    for row in rows {
        let (invocation_id, value) = row.map_err(|e| OrbitError::Store(e.to_string()))?;
        grouped.entry(invocation_id).or_default().push(value);
    }
    Ok(grouped)
}

fn sql_placeholders(count: usize) -> String {
    (0..count)
        .map(|index| format!("?{}", index + 1))
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Default)]
struct InvocationListQuery {
    conditions: Vec<String>,
    params: Vec<Box<dyn ToSql>>,
}

impl InvocationListQuery {
    fn push_filter<T>(&mut self, sql: &str, value: T)
    where
        T: ToSql + 'static,
    {
        self.push_value(value);
        self.conditions.push(format!("{sql}{}", self.len()));
    }

    fn push_value<T>(&mut self, value: T)
    where
        T: ToSql + 'static,
    {
        self.params.push(Box::new(value));
    }

    fn len(&self) -> usize {
        self.params.len()
    }

    fn where_clause(&self) -> String {
        if self.conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", self.conditions.join(" AND "))
        }
    }
}

impl Store {
    fn connection_handle(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, rusqlite::Connection>, OrbitError> {
        self.conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use orbit_common::types::{InvocationTrace, RoleSlot};

    use super::*;

    #[test]
    fn invocation_records_persist_planning_duel_slot() {
        let store = Store::open_in_memory().expect("open store");

        store
            .insert_invocation_trace_record(&InvocationInsertParams {
                job_run_id: "jrun-1".to_string(),
                activity_id: "propose_duel_plan".to_string(),
                agent: "gemini".to_string(),
                model: Some("gemini-3.1-pro".to_string()),
                slot: Some(RoleSlot::PlannerA),
                task_ids: vec!["ORB-1".to_string()],
                trace: InvocationTrace::default(),
            })
            .expect("insert invocation");

        let records = store
            .list_invocation_records(&InvocationQuery {
                job_run_id: Some("jrun-1".to_string()),
                slot: Some(RoleSlot::PlannerA),
                limit: 10,
                ..InvocationQuery::default()
            })
            .expect("list records");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].slot, Some(RoleSlot::PlannerA));
    }

    #[test]
    fn invocation_records_persist_non_duel_slot_as_null() {
        let store = Store::open_in_memory().expect("open store");

        store
            .insert_invocation_trace_record(&InvocationInsertParams {
                job_run_id: "jrun-2".to_string(),
                activity_id: "implement_one".to_string(),
                agent: "codex".to_string(),
                model: Some("gpt-5.5".to_string()),
                slot: None,
                task_ids: vec!["ORB-2".to_string()],
                trace: InvocationTrace::default(),
            })
            .expect("insert invocation");

        let records = store
            .list_invocation_records(&InvocationQuery {
                job_run_id: Some("jrun-2".to_string()),
                limit: 10,
                ..InvocationQuery::default()
            })
            .expect("list records");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].slot, None);
    }
}
