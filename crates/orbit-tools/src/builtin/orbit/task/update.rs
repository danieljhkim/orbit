use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitTaskUpdateTool;

impl Tool for OrbitTaskUpdateTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::super::orbit_id_params("task");
        parameters.extend([
            ToolParam {
                name: "title".to_string(),
                description: "New task title".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "description".to_string(),
                description: "New task description (empty string clears)".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "acceptance_criteria".to_string(),
                description: "New acceptance criteria as an array of strings or a single string"
                    .to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
            ToolParam {
                name: "dependencies".to_string(),
                description: "Replacement dependency task IDs as a string or array of strings"
                    .to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
            ToolParam {
                name: "relations".to_string(),
                description: "Replacement typed task relations as an array of {type, target} objects"
                    .to_string(),
                param_type: "array".to_string(),
                required: false,
            },
            ToolParam {
                name: "tags".to_string(),
                description: "Replacement task tags as a string or array of strings".to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
            ToolParam {
                name: "plan".to_string(),
                description: "Replacement task plan text (empty string clears)".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "status".to_string(),
                description: "New task status".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "type".to_string(),
                description: "New task type".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "source_task_id".to_string(),
                description: "For bug tasks: originating task ID that introduced the defect (empty string clears)"
                    .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "planned_by".to_string(),
                description: "Explicit planning attribution label (empty string clears)"
                    .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "implemented_by".to_string(),
                description: "Explicit implementation attribution label (empty string clears)"
                    .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "execution_summary".to_string(),
                description: "Replacement execution summary text".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "comment".to_string(),
                description: "Task comment to append".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "pr_status".to_string(),
                description: "PR review status (e.g. approve, request-changes)".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "job_run_id".to_string(),
                description: "Job run ID to associate with the task (empty string clears)"
                    .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "crew".to_string(),
                description: "Named crew to use when running this task (empty string clears)"
                    .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "context_files".to_string(),
                description:
                    "Task context selectors as a comma-separated string or array of strings. Prefer canonical selectors: `file:path`, `dir:path`, or `symbol:path#name:kind`. Legacy raw paths are accepted and upgraded automatically."
                        .to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
            ToolParam {
                name: "artifacts".to_string(),
                description:
                    "Task artifacts to write under `artifacts/`. Accepts either an object \
                    map of `path -> content` or an array of `{ path, content }` objects."
                        .to_string(),
                param_type: "object".to_string(),
                required: false,
            },
        ]);
        parameters.extend(super::super::model_identity_params());

        ToolSchema {
            name: "orbit.task.update".to_string(),
            description: "Update an Orbit task and return the fresh task JSON".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::reject_agent_field(&input, "orbit.task.update")?;
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::TaskUpdate)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::{Value, json};

    use crate::{OrbitTaskScope, OrbitToolHost};

    use super::*;

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
}
