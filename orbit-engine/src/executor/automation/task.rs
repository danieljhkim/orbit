use orbit_types::{OrbitError, TaskStatus};
use serde_json::{Value, json};

use crate::context::TaskHost;

use super::input::{input_string_field, required_input_string};

pub(super) fn update_task<H: TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;

    // Tolerate missing status: when the upstream agent persisted status directly
    // to the task (via orbit.task.update) but crashed before returning structured
    // output, piped input will lack `status`. In that case, treat as a no-op
    // since the task already has the correct status.
    let status = match input_string_field(input, "status") {
        Some(raw) => raw
            .parse::<TaskStatus>()
            .map_err(|error| OrbitError::InvalidInput(format!("invalid input.status: {error}")))?,
        None => return Ok(json!({})),
    };

    // Idempotent: if task is already at the target status, skip the update.
    let task = host.get_task(task_id)?;
    if task.status == status {
        return Ok(json!({}));
    }

    // Safety net: if execution_summary is present in input (e.g. from the
    // implement_change output), persist it so the summary is not lost even when
    // the agent's own orbit.task.update call was skipped or failed.
    let execution_summary = input_string_field(input, "execution_summary");
    let note = input_string_field(input, "note")
        .or_else(|| Some(format!("automation: update_task → {status}")));
    host.update_task_from_activity(task_id, status, execution_summary, None, note)?;
    Ok(json!({}))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use chrono::Utc;
    use orbit_types::{ActorIdentity, OrbitError, Task, TaskPriority, TaskStatus, TaskType};
    use serde_json::json;

    use super::*;
    use crate::context::{TaskAutomationUpdate, TaskHost};

    struct FakeHost {
        task: RefCell<Option<Task>>,
        updated_status: RefCell<Option<TaskStatus>>,
    }

    impl FakeHost {
        fn new(task: Task) -> Self {
            Self {
                task: RefCell::new(Some(task)),
                updated_status: RefCell::new(None),
            }
        }
    }

    impl TaskHost for FakeHost {
        fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
            self.task
                .borrow()
                .clone()
                .filter(|t| t.id == task_id)
                .ok_or_else(|| OrbitError::TaskNotFound(task_id.to_string()))
        }

        fn start_task(
            &self,
            _task_id: &str,
            _note: Option<String>,
            _comment: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!()
        }

        fn update_task_from_activity(
            &self,
            _task_id: &str,
            status: TaskStatus,
            _execution_summary: Option<String>,
            _comment: Option<String>,
            _note: Option<String>,
        ) -> Result<Task, OrbitError> {
            *self.updated_status.borrow_mut() = Some(status);
            self.task
                .borrow()
                .clone()
                .ok_or_else(|| OrbitError::TaskNotFound("".to_string()))
        }

        fn apply_task_automation_update(
            &self,
            _task_id: &str,
            _update: TaskAutomationUpdate,
        ) -> Result<(), OrbitError> {
            Ok(())
        }
    }

    fn test_task() -> Task {
        Task {
            id: "T20260328-001".to_string(),
            parent_id: None,
            title: "test task".to_string(),
            description: String::new(),
            plan: String::new(),
            execution_summary: String::new(),
            context_files: vec![],
            workspace_path: None,
            repo_root: None,
            assigned_to: None,
            created_by: None,
            actor_identity: ActorIdentity::System,
            status: TaskStatus::InProgress,
            priority: TaskPriority::Medium,
            task_type: TaskType::Task,
            pr_number: None,
            pr_status: None,
            proposed_by: None,
            source_task_id: None,
            complexity: None,
            comments: vec![],
            history: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn missing_status_is_noop() {
        let host = FakeHost::new(test_task());
        let result = update_task(&host, &json!({ "task_id": "T20260328-001" }))
            .expect("should succeed as no-op");
        assert_eq!(result, json!({}));
        assert!(
            host.updated_status.borrow().is_none(),
            "host should not be called"
        );
    }

    #[test]
    fn present_status_updates_task() {
        let host = FakeHost::new(test_task());
        let result = update_task(
            &host,
            &json!({ "task_id": "T20260328-001", "status": "review" }),
        )
        .expect("should succeed");
        assert_eq!(result, json!({}));
        assert_eq!(*host.updated_status.borrow(), Some(TaskStatus::Review));
    }

    #[test]
    fn idempotent_when_already_at_target_status() {
        let host = FakeHost::new(test_task()); // status is InProgress
        let result = update_task(
            &host,
            &json!({ "task_id": "T20260328-001", "status": "in-progress" }),
        )
        .expect("should succeed");
        assert_eq!(result, json!({}));
        assert!(host.updated_status.borrow().is_none(), "should skip update");
    }
}
