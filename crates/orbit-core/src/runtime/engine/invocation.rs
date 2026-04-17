use orbit_store::{
    ActivityInvocationMetrics, AgentInvocationMetrics, InvocationQuery, InvocationRecord, Store,
    TaskInvocationMetrics, ToolInvocationMetrics,
};
use orbit_types::OrbitError;
use serde_json::Value;

use crate::OrbitRuntime;

pub(super) fn open_invocation_store(runtime: &OrbitRuntime) -> Result<Store, OrbitError> {
    Store::open(&runtime.context.persistence().audit_db)
}

pub(super) fn associated_task_ids(input: &Value) -> Vec<String> {
    let mut task_ids = Vec::new();
    if let Some(task_id) = input.get("task_id").and_then(Value::as_str) {
        push_unique_task_id(&mut task_ids, task_id);
    }
    if let Some(items) = input.get("task_ids").and_then(Value::as_array) {
        for item in items {
            if let Some(task_id) = item.as_str() {
                push_unique_task_id(&mut task_ids, task_id);
            }
        }
    }
    if let Some(items) = input.get("tasks").and_then(Value::as_array) {
        for item in items {
            if let Some(task_id) = item.as_str() {
                push_unique_task_id(&mut task_ids, task_id);
                continue;
            }
            if let Some(task_id) = item
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| item.get("task_id").and_then(Value::as_str))
            {
                push_unique_task_id(&mut task_ids, task_id);
            }
        }
    }
    task_ids
}

impl OrbitRuntime {
    pub fn activity_invocation_metrics(
        &self,
    ) -> Result<Vec<ActivityInvocationMetrics>, OrbitError> {
        open_invocation_store(self)?.list_activity_invocation_metrics()
    }

    pub fn agent_invocation_metrics(&self) -> Result<Vec<AgentInvocationMetrics>, OrbitError> {
        open_invocation_store(self)?.list_agent_invocation_metrics()
    }

    pub fn task_invocation_metrics(
        &self,
        task_id: &str,
    ) -> Result<TaskInvocationMetrics, OrbitError> {
        open_invocation_store(self)?.get_task_invocation_metrics(task_id)
    }

    pub fn tool_invocation_metrics(&self) -> Result<Vec<ToolInvocationMetrics>, OrbitError> {
        open_invocation_store(self)?.list_tool_invocation_metrics()
    }

    pub fn invocation_records(
        &self,
        query: InvocationQuery,
    ) -> Result<Vec<InvocationRecord>, OrbitError> {
        open_invocation_store(self)?.list_invocation_records(&query)
    }
}

fn push_unique_task_id(task_ids: &mut Vec<String>, task_id: &str) {
    let task_id = task_id.trim();
    if !task_id.is_empty() && !task_ids.iter().any(|existing| existing == task_id) {
        task_ids.push(task_id.to_string());
    }
}
