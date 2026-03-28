use orbit_types::OrbitError;
use serde_json::{Value, json};

use super::input::required_input_string;
use super::review::normalize_review_decision;
use crate::context::TaskHost;

pub(super) fn check_review_decision<H: TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let task = host.get_task(task_id)?;

    if task.pr_number.is_none() {
        return Ok(json!({ "review_decision": "SKIPPED" }));
    }

    let pr_status = task.pr_status.clone().unwrap_or_else(|| "none".to_string());

    let normalized = normalize_review_decision(&pr_status);
    if normalized == "APPROVED" {
        Ok(json!({ "review_decision": normalized }))
    } else {
        Err(OrbitError::Execution(format!(
            "task '{task_id}' is not approved (pr_status={pr_status})"
        )))
    }
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
    }

    impl FakeHost {
        fn new(task: Task) -> Self {
            Self {
                task: RefCell::new(Some(task)),
            }
        }
    }

    impl TaskHost for FakeHost {
        fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
            let task = self
                .task
                .borrow()
                .clone()
                .ok_or_else(|| OrbitError::TaskNotFound(task_id.to_string()))?;
            if task.id != task_id {
                return Err(OrbitError::TaskNotFound(task_id.to_string()));
            }
            Ok(task)
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
            _status: TaskStatus,
            _execution_summary: Option<String>,
            _comment: Option<String>,
            _note: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!()
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
            status: TaskStatus::Review,
            priority: TaskPriority::Medium,
            task_type: TaskType::Issue,
            pr_number: Some("42".to_string()),
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
    fn reads_from_task_even_when_input_has_different_pr_status() {
        let mut task = test_task();
        task.pr_status = Some("approve".to_string());
        let host = FakeHost::new(task);

        // Input has stale "request-changes" but task has "approve" — should succeed.
        let result = check_review_decision(
            &host,
            &json!({
                "task_id": "T20260328-001",
                "pr_status": "request-changes",
            }),
        )
        .expect("should succeed because task.pr_status is approve");

        assert_eq!(result["review_decision"], json!("APPROVED"));
    }

    #[test]
    fn fails_when_task_pr_status_is_request_changes_even_if_input_has_approve() {
        let mut task = test_task();
        task.pr_status = Some("request-changes".to_string());
        let host = FakeHost::new(task);

        let err = check_review_decision(
            &host,
            &json!({
                "task_id": "T20260328-001",
                "pr_status": "approve",
            }),
        )
        .expect_err("should fail because task.pr_status is request-changes");

        assert!(err.to_string().contains("not approved"));
        assert!(err.to_string().contains("request-changes"));
    }

    #[test]
    fn fails_when_task_pr_status_is_none() {
        let task = test_task(); // pr_status is None by default
        let host = FakeHost::new(task);

        let err = check_review_decision(
            &host,
            &json!({
                "task_id": "T20260328-001",
            }),
        )
        .expect_err("should fail when pr_status is None");

        assert!(err.to_string().contains("not approved"));
        assert!(err.to_string().contains("pr_status=none"));
    }

    #[test]
    fn skips_when_task_has_no_pr_number() {
        let mut task = test_task();
        task.pr_number = None;
        let host = FakeHost::new(task);

        let result = check_review_decision(
            &host,
            &json!({
                "task_id": "T20260328-001",
            }),
        )
        .expect("should succeed with SKIPPED when no pr_number");

        assert_eq!(result["review_decision"], json!("SKIPPED"));
    }
}
