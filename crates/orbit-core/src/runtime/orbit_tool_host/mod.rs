mod input;
mod json;

use std::collections::BTreeSet;
use std::str::FromStr;
use std::sync::Arc;

use orbit_store::state_io;
use orbit_tools::{OrbitBuiltinAction, OrbitTaskScope, OrbitToolHost};
use orbit_types::{
    OrbitError, ReviewThreadStatus, TaskPriority, TaskStatus, TaskType,
    optional_csv_or_string_list_alias, optional_raw_string, optional_string, optional_string_alias,
    optional_string_list_alias, required_string, split_csv,
};
use serde_json::{Value, json};

use self::input::{
    empty_string_to_none, parse_artifacts, parse_task_complexity, parse_task_priority,
    parse_task_status, parse_task_type, resolve_state_dir, resolve_state_payload,
    resolve_step_index,
};
use self::json::{
    serialize_error, serialize_task, serialize_task_lint_report, task_fields_to_json,
    task_lock_status_rank, task_lock_to_json, task_to_json,
};
use crate::OrbitRuntime;
use crate::command::task::{TaskAddParams, TaskUpdateParams};

pub(crate) fn build_orbit_tool_host(
    runtime: &OrbitRuntime,
    task_id: Option<String>,
) -> Arc<dyn OrbitToolHost> {
    Arc::new(RuntimeOrbitToolHost {
        runtime: runtime.clone(),
        task_scope: OrbitTaskScope {
            orbit_root: Some(runtime.data_root_path().to_path_buf()),
            task_id,
        },
    })
}

#[derive(Clone)]
struct RuntimeOrbitToolHost {
    runtime: OrbitRuntime,
    task_scope: OrbitTaskScope,
}

impl OrbitToolHost for RuntimeOrbitToolHost {
    fn execute(
        &self,
        action: OrbitBuiltinAction,
        input: Value,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<Value, OrbitError> {
        match action {
            OrbitBuiltinAction::ActivityShow => {
                let id = required_string(&input, &["id"], "id")?;
                serde_json::to_value(self.runtime.show_activity(&id)?)
                    .map_err(serialize_error("serialize activity"))
            }
            OrbitBuiltinAction::ReviewThreadAdd => {
                let id = required_string(&input, &["id"], "id")?;
                let body = required_string(&input, &["body"], "body")?;
                let path = optional_string(&input, "path")?;
                let line = optional_string(&input, "line")?
                    .map(|value| {
                        value.parse::<u64>().map_err(|error| {
                            OrbitError::InvalidInput(format!(
                                "`line` must be an unsigned integer: {error}"
                            ))
                        })
                    })
                    .transpose()?;
                self.runtime
                    .add_review_thread(&id, body, path, line, agent, model)?;
                serialize_task(&self.runtime.get_task(&id)?)
            }
            OrbitBuiltinAction::ReviewThreadList => {
                let id = required_string(&input, &["id"], "id")?;
                let status = optional_string(&input, "status")?
                    .map(|value| ReviewThreadStatus::from_str(&value))
                    .transpose()
                    .map_err(OrbitError::InvalidInput)?;
                serde_json::to_value(self.runtime.list_review_threads(&id, status)?)
                    .map_err(serialize_error("serialize review threads"))
            }
            OrbitBuiltinAction::ReviewThreadReply => {
                let id = required_string(&input, &["id"], "id")?;
                let thread_id = required_string(&input, &["thread_id"], "thread_id")?;
                let body = required_string(&input, &["body"], "body")?;
                self.runtime
                    .reply_review_thread(&id, &thread_id, body, agent, model)?;
                serialize_task(&self.runtime.get_task(&id)?)
            }
            OrbitBuiltinAction::ReviewThreadResolve => {
                let id = required_string(&input, &["id"], "id")?;
                let thread_id = required_string(&input, &["thread_id"], "thread_id")?;
                self.runtime
                    .resolve_review_thread(&id, &thread_id, agent, model)?;
                serialize_task(&self.runtime.get_task(&id)?)
            }
            OrbitBuiltinAction::StateGet => {
                let state_dir = resolve_state_dir(&self.task_scope, &input)?;
                let pipeline = state_io::read_pipeline(&state_dir)?;
                match optional_string(&input, "key")? {
                    Some(key) => Ok(pipeline
                        .as_object()
                        .and_then(|map| map.get(&key))
                        .cloned()
                        .unwrap_or(Value::Null)),
                    None => Ok(pipeline),
                }
            }
            OrbitBuiltinAction::StateSet => {
                let state_dir = resolve_state_dir(&self.task_scope, &input)?;
                let step_index = resolve_step_index(&input)?;
                let payload = resolve_state_payload(&input)?;
                state_io::write_step_output(&state_dir, step_index, &payload)?;
                Ok(json!({
                    "state_dir": state_dir.display().to_string(),
                    "step_index": step_index,
                    "written": payload,
                }))
            }
            OrbitBuiltinAction::TaskAdd => {
                let title = required_string(&input, &["title"], "title")?;
                let description = required_string(&input, &["description"], "description")?;
                let workspace = required_string(&input, &["workspace"], "workspace")?;
                let plan = match input.get("plan") {
                    Some(Value::String(value)) => value.clone(),
                    Some(Value::Null) | None => String::new(),
                    Some(_) => {
                        return Err(OrbitError::InvalidInput(
                            "`plan` must be a string".to_string(),
                        ));
                    }
                };
                let task = self.runtime.add_task_with_identity(
                    TaskAddParams {
                        parent_id: optional_string_alias(
                            &input,
                            &["parent_id", "parent", "parentId"],
                        )?,
                        title,
                        description,
                        acceptance_criteria: optional_string_list_alias(
                            &input,
                            &[
                                "acceptance_criteria",
                                "acceptanceCriteria",
                                "acceptance-criteria",
                            ],
                        )?
                        .unwrap_or_default(),
                        plan,
                        comment: optional_string(&input, "comment")?,
                        context_files: optional_string(&input, "context")?
                            .map(|value| split_csv(&value))
                            .unwrap_or_default(),
                        workspace_path: Some(workspace),
                        priority: optional_string(&input, "priority")?
                            .map(|value| parse_task_priority("priority", &value))
                            .transpose()?
                            .unwrap_or(TaskPriority::Medium),
                        complexity: optional_string(&input, "complexity")?
                            .map(|value| parse_task_complexity("complexity", &value))
                            .transpose()?,
                        task_type: optional_string_alias(
                            &input,
                            &["type", "task_type", "taskType"],
                        )?
                        .map(|value| parse_task_type("type", &value))
                        .transpose()?
                        .unwrap_or(TaskType::Task),
                        system_created: false,
                        source_task_id: optional_string_alias(
                            &input,
                            &["source_task_id", "source_task", "sourceTaskId"],
                        )?,
                    },
                    agent,
                    model,
                )?;
                serialize_task(&task)
            }
            OrbitBuiltinAction::TaskApprove => {
                let id = required_string(&input, &["id"], "id")?;
                let task = self.runtime.approve_task_with_identity(
                    &id,
                    optional_string(&input, "note")?,
                    optional_string(&input, "comment")?,
                    agent,
                    model,
                )?;
                serialize_task(&task)
            }
            OrbitBuiltinAction::TaskDelete => {
                let id = required_string(&input, &["id"], "id")?;
                self.runtime.delete_task(&id)?;
                Ok(json!({ "id": id, "deleted": true }))
            }
            OrbitBuiltinAction::TaskLint => {
                let id = required_string(&input, &["id"], "id")?;
                serialize_task_lint_report(&self.runtime.lint_task(&id)?)
            }
            OrbitBuiltinAction::TaskList => {
                let status = optional_string(&input, "status")?
                    .map(|value| parse_task_status("status", &value))
                    .transpose()?;
                let parent_id =
                    optional_string_alias(&input, &["parent_id", "parent", "parentId"])?;
                let batch_id = optional_string(&input, "batch_id")?;
                let tasks = self.runtime.list_tasks_filtered(
                    status,
                    None,
                    parent_id.as_deref(),
                    batch_id.as_deref(),
                )?;
                Ok(Value::Array(
                    tasks.into_iter().map(task_to_json).collect::<Vec<_>>(),
                ))
            }
            OrbitBuiltinAction::TaskLocks => {
                let mut tasks: Vec<_> = self
                    .runtime
                    .list_tasks()?
                    .into_iter()
                    .filter(|task| {
                        matches!(task.status, TaskStatus::InProgress | TaskStatus::Review)
                    })
                    .collect();
                tasks.sort_by_key(|task| {
                    (
                        task_lock_status_rank(task.status),
                        task.created_at,
                        task.id.clone(),
                    )
                });

                let locked_files: BTreeSet<String> = tasks
                    .iter()
                    .flat_map(|task| task.context_files.iter().cloned())
                    .collect();

                Ok(json!({
                    "locked_files": locked_files.iter().cloned().collect::<Vec<_>>(),
                    "by_task": tasks.iter().map(task_lock_to_json).collect::<Vec<_>>(),
                    "total_locked": locked_files.len(),
                    "total_tasks": tasks.len(),
                }))
            }
            OrbitBuiltinAction::TaskReject => {
                let id = required_string(&input, &["id"], "id")?;
                let note = required_string(&input, &["note"], "note")?;
                let task = self.runtime.reject_task_with_identity(
                    &id,
                    note,
                    optional_string(&input, "comment")?,
                    agent,
                    model,
                )?;
                serialize_task(&task)
            }
            OrbitBuiltinAction::TaskShow => {
                let id = required_string(&input, &["id"], "id")?;
                let task = self.runtime.get_task(&id)?;
                let fields = optional_csv_or_string_list_alias(&input, &["fields", "field"])?;
                if let Some(fields) = fields {
                    task_fields_to_json(&self.runtime, &task, &fields)
                } else {
                    serialize_task(&task)
                }
            }
            OrbitBuiltinAction::TaskStart => {
                let id = required_string(&input, &["id"], "id")?;
                let task = self.runtime.start_task_with_identity(
                    &id,
                    optional_string(&input, "note")?,
                    optional_string(&input, "comment")?,
                    agent,
                    model,
                )?;
                serialize_task(&task)
            }
            OrbitBuiltinAction::TaskUpdate => {
                let id = required_string(&input, &["id"], "id")?;
                let task = self.runtime.update_task_with_identity(
                    &id,
                    TaskUpdateParams {
                        title: optional_string(&input, "title")?,
                        description: input
                            .get("description")
                            .map(|value| {
                                value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                                    OrbitError::InvalidInput(
                                        "`description` must be a string".to_string(),
                                    )
                                })
                            })
                            .transpose()?,
                        acceptance_criteria: optional_string_list_alias(
                            &input,
                            &[
                                "acceptance_criteria",
                                "acceptanceCriteria",
                                "acceptance-criteria",
                            ],
                        )?,
                        plan: input
                            .get("plan")
                            .map(|value| {
                                value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                                    OrbitError::InvalidInput("`plan` must be a string".to_string())
                                })
                            })
                            .transpose()?,
                        execution_summary: optional_raw_string(&input, "execution_summary")?,
                        comment: optional_string(&input, "comment")?,
                        status: optional_string(&input, "status")?
                            .map(|value| parse_task_status("status", &value))
                            .transpose()?,
                        pr_number: optional_raw_string(&input, "pr_number")?
                            .map(empty_string_to_none),
                        pr_status: optional_raw_string(&input, "pr_status")?
                            .map(empty_string_to_none),
                        batch_id: optional_raw_string(&input, "batch_id")?
                            .map(empty_string_to_none),
                        context_files: optional_csv_or_string_list_alias(
                            &input,
                            &["context_files"],
                        )?,
                        upsert_artifacts: parse_artifacts(&input)?,
                        ..Default::default()
                    },
                    agent,
                    model,
                )?;
                serialize_task(&task)
            }
        }
    }

    fn task_scope(&self) -> OrbitTaskScope {
        self.task_scope.clone()
    }
}
