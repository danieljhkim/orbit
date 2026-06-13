#![allow(missing_docs)]

use std::collections::HashMap;
use std::path::Path;

use orbit_common::types::{
    Activity, Job, JobTargetType, OrbitError, OrbitEvent, Role, Task, TaskArtifact, TaskPriority,
    TaskStatus,
};
use orbit_tools::ToolContext;
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::context::{
    EnvironmentHost, JobRunResult, RuntimeHost, TaskActivityUpdate, TaskAutomationUpdate,
    TaskReadHost, TaskWriteHost,
};
use crate::executor::registry::ActivityExecutorRegistry;

struct CommandTestHost {
    data_root: TempDir,
    scoreboard_dir: TempDir,
    registry: ActivityExecutorRegistry,
}

impl CommandTestHost {
    fn new() -> Self {
        Self {
            data_root: tempfile::tempdir().expect("create data root"),
            scoreboard_dir: tempfile::tempdir().expect("create scoreboard dir"),
            registry: ActivityExecutorRegistry::default(),
        }
    }
}

impl EnvironmentHost for CommandTestHost {
    fn agent_provider_config(&self) -> HashMap<String, String> {
        HashMap::new()
    }

    fn execution_env_inherit(&self) -> bool {
        false
    }

    fn hydrated_env_allowlist(&self, _env_extra: &[String]) -> Vec<(String, String)> {
        Vec::new()
    }

    fn orbit_root(&self) -> Option<String> {
        None
    }

    fn cli_command_environment(&self, _env_extra: &[String]) -> Vec<(String, String)> {
        Vec::new()
    }

    fn missing_required_environment_vars(&self, _required_env_vars: &[&str]) -> Vec<String> {
        Vec::new()
    }
}

impl RuntimeHost for CommandTestHost {
    fn record_event(&self, _event: OrbitEvent) -> Result<(), OrbitError> {
        Ok(())
    }

    fn repo_root(&self) -> Result<String, OrbitError> {
        Ok(self.data_root.path().to_string_lossy().into_owned())
    }

    fn data_root(&self) -> &Path {
        self.data_root.path()
    }

    fn activity_executor_registry(&self) -> &ActivityExecutorRegistry {
        &self.registry
    }

    fn run_job_now_with_input_debug(
        &self,
        _job_id: &str,
        _input: Value,
        _debug: bool,
    ) -> Result<JobRunResult, OrbitError> {
        Err(OrbitError::Execution(
            "run_job_now_with_input_debug is not needed by command tests".to_string(),
        ))
    }

    fn validate_activity_target_exists(
        &self,
        _target_type: JobTargetType,
        _target_id: &str,
    ) -> Result<Activity, OrbitError> {
        Err(OrbitError::Execution(
            "validate_activity_target_exists is not needed by command tests".to_string(),
        ))
    }

    fn get_job(&self, _job_id: &str) -> Result<Option<Job>, OrbitError> {
        Ok(None)
    }

    fn run_tool_with_context_and_role(
        &self,
        _name: &str,
        _input: Value,
        _role: Role,
        _tool_context: ToolContext,
    ) -> Result<Value, OrbitError> {
        Err(OrbitError::Execution(
            "run_tool_with_context_and_role is not needed by command tests".to_string(),
        ))
    }

    fn maybe_create_failure_task(
        &self,
        _job_id: &str,
        _run_id: &str,
        _error_code: &str,
        _error_message: &str,
        _agent: Option<&str>,
        _model: Option<&str>,
    ) -> Result<(), OrbitError> {
        Ok(())
    }

    fn scoring_enabled(&self) -> bool {
        false
    }

    fn graph_editing(&self) -> bool {
        false
    }

    fn scoreboard_dir(&self) -> &Path {
        self.scoreboard_dir.path()
    }
}

impl TaskReadHost for CommandTestHost {
    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
        Err(OrbitError::Execution(format!(
            "get_task is not needed by command tests for '{task_id}'"
        )))
    }

    fn get_task_artifacts(&self, _task_id: &str) -> Result<Vec<TaskArtifact>, OrbitError> {
        Ok(Vec::new())
    }

    fn list_tasks_filtered(
        &self,
        _status: Option<TaskStatus>,
        _priority: Option<TaskPriority>,
        _parent_id: Option<&str>,
        _job_run_id: Option<&str>,
        _external_ref: Option<&orbit_common::types::ExternalRef>,
        _has_external_ref_system: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        Ok(Vec::new())
    }
}

impl TaskWriteHost for CommandTestHost {
    fn start_task(
        &self,
        _task_id: &str,
        _note: Option<String>,
        _comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        Err(OrbitError::Execution(
            "start_task is not needed by command tests".to_string(),
        ))
    }

    fn admit_task_for_workflow(&self, _task_id: &str, _workflow: &str) -> Result<Task, OrbitError> {
        Err(OrbitError::Execution(
            "admit_task_for_workflow is not needed by command tests".to_string(),
        ))
    }

    fn update_task_from_activity(
        &self,
        _task_id: &str,
        _update: TaskActivityUpdate,
    ) -> Result<Task, OrbitError> {
        Err(OrbitError::Execution(
            "update_task_from_activity is not needed by command tests".to_string(),
        ))
    }

    fn apply_task_automation_update(
        &self,
        _task_id: &str,
        _update: TaskAutomationUpdate,
    ) -> Result<(), OrbitError> {
        Err(OrbitError::Execution(
            "apply_task_automation_update is not needed by command tests".to_string(),
        ))
    }
}

#[test]
fn templated_semicolon_payload_is_not_executed_by_shell() {
    let host = CommandTestHost::new();
    let sentinel = host.data_root().join("pwned-by-template");
    let payload = format!("safe; touch {}", sentinel.to_string_lossy());
    let input = json!({
        "command": "printf '%s\\n' {{ input.title }}",
        "title": payload,
    });

    let result =
        super::run_command(&host, &input, &HashMap::new(), None).expect("run_command succeeds");

    assert_eq!(result, json!({ "exit_code": 0 }));
    assert!(
        !sentinel.exists(),
        "template value was re-parsed as shell syntax"
    );
}

#[test]
fn templated_command_substitution_payload_is_not_executed_by_shell() {
    let host = CommandTestHost::new();
    let sentinel = host.data_root().join("pwned-by-command-substitution");
    let payload = format!("$(touch {})", sentinel.to_string_lossy());
    let input = json!({
        "command": "printf '%s\\n' {{ input.title }}",
        "title": payload,
    });

    let result =
        super::run_command(&host, &input, &HashMap::new(), None).expect("run_command succeeds");

    assert_eq!(result, json!({ "exit_code": 0 }));
    assert!(
        !sentinel.exists(),
        "template value was re-parsed as shell syntax"
    );
}

#[test]
fn single_quoted_template_value_still_substitutes() {
    let host = CommandTestHost::new();
    let input = json!({
        "command": "test '{{ input.title }}' = 'value with spaces'",
        "title": "value with spaces",
    });

    let result =
        super::run_command(&host, &input, &HashMap::new(), None).expect("run_command succeeds");

    assert_eq!(result, json!({ "exit_code": 0 }));
}
