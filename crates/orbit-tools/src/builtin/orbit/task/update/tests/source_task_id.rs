//! Tests for source_task_id handling in task.update (mirrors add semantics).

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::{OrbitTaskScope, OrbitToolHost};

use super::super::*;

#[derive(Debug, Clone)]
struct FakeTask {
    id: String,
    source_task_id: Option<String>,
    updated_at: String,
    history: Vec<Value>,
}

struct FakeTaskHost {
    task: Mutex<FakeTask>,
}

impl FakeTaskHost {
    fn seeded(source_task_id: Option<&str>) -> Self {
        Self {
            task: Mutex::new(FakeTask {
                id: "ORB-00001".to_string(),
                source_task_id: source_task_id.map(ToOwned::to_owned),
                updated_at: "2026-05-17T00:00:00Z".to_string(),
                history: Vec::new(),
            }),
        }
    }
}

impl OrbitToolHost for FakeTaskHost {
    fn execute(
        &self,
        action: OrbitBuiltinAction,
        input: Value,
        _agent: Option<String>,
        _model: Option<String>,
        _reservation_owner: Option<crate::ReservationOwnerContext>,
    ) -> Result<Value, OrbitError> {
        assert_eq!(action, OrbitBuiltinAction::TaskUpdate);
        let id = input.get("id").and_then(Value::as_str).expect("id");
        let mut task = self.task.lock().expect("task lock");
        assert_eq!(id, task.id);

        if let Some(value) = input.get("source_task_id") {
            let raw = value.as_str().ok_or_else(|| {
                OrbitError::InvalidInput("`source_task_id` must be a string".to_string())
            })?;
            let next_source_task_id = (!raw.is_empty()).then(|| raw.to_string());
            if task.source_task_id != next_source_task_id {
                task.updated_at = "2026-05-17T00:00:01Z".to_string();
                task.history.push(json!({
                    "event": "updated",
                    "note": "source_task_id changed",
                }));
            }
            task.source_task_id = next_source_task_id;
        }

        Ok(json!({
            "id": task.id.clone(),
            "type": "bug",
            "source_task_id": task.source_task_id.clone(),
            "updated_at": task.updated_at.clone(),
            "history": task.history.clone(),
        }))
    }

    fn task_scope(&self) -> OrbitTaskScope {
        OrbitTaskScope::default()
    }
}

fn update_tool_context(host: Arc<FakeTaskHost>) -> ToolContext {
    ToolContext {
        orbit_host: Some(host),
        ..ToolContext::default()
    }
}

#[test]
fn schema_exposes_source_task_id() {
    let schema = OrbitTaskUpdateTool.schema();

    let param = schema
        .parameters
        .iter()
        .find(|param| param.name == "source_task_id")
        .expect("source_task_id param");

    assert_eq!(param.param_type, "string");
    assert!(!param.required);
    assert!(param.description.contains("originating task ID"));
}

#[test]
fn update_handler_persists_source_task_id() {
    let host = Arc::new(FakeTaskHost::seeded(None));
    let output = OrbitTaskUpdateTool
        .execute(
            &update_tool_context(Arc::clone(&host)),
            json!({
                "id": "ORB-00001",
                "model": "codex",
                "source_task_id": "ORB-00000",
            }),
        )
        .expect("update succeeds");

    assert_eq!(output.get("type").and_then(Value::as_str), Some("bug"));
    assert_eq!(
        output.get("source_task_id").and_then(Value::as_str),
        Some("ORB-00000")
    );
    assert_eq!(
        host.task
            .lock()
            .expect("task lock")
            .source_task_id
            .as_deref(),
        Some("ORB-00000")
    );
}

#[test]
fn update_handler_clears_source_task_id_with_empty_string() {
    let host = Arc::new(FakeTaskHost::seeded(Some("ORB-00000")));
    let output = OrbitTaskUpdateTool
        .execute(
            &update_tool_context(Arc::clone(&host)),
            json!({
                "id": "ORB-00001",
                "model": "codex",
                "source_task_id": "",
            }),
        )
        .expect("update succeeds");

    assert_eq!(output.get("source_task_id"), Some(&Value::Null));
    assert_eq!(host.task.lock().expect("task lock").source_task_id, None);
}

#[test]
fn update_handler_stores_unresolved_source_task_id_like_add() {
    let host = Arc::new(FakeTaskHost::seeded(None));
    let output = OrbitTaskUpdateTool
        .execute(
            &update_tool_context(Arc::clone(&host)),
            json!({
                "id": "ORB-00001",
                "model": "codex",
                "source_task_id": "ORB-99999",
            }),
        )
        .expect("loose source reference is stored");

    assert_eq!(
        output.get("source_task_id").and_then(Value::as_str),
        Some("ORB-99999")
    );
}
