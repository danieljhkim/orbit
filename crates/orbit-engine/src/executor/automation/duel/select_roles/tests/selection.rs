use std::path::{Path, PathBuf};

use orbit_common::types::{
    Activity, AgentModelPair, ExternalRef, Job, JobTargetType, OrbitError, OrbitEvent, Role, Task,
    TaskArtifact, TaskPriority, TaskStatus,
};
use orbit_store::InvocationRecord;
use orbit_tools::ToolContext;
use serde_json::{Value, json};

use crate::context::{
    JobRunResult, RuntimeHost, TaskActivityUpdate, TaskAutomationUpdate, TaskReadHost,
    TaskWriteHost,
};
use crate::executor::registry::ActivityExecutorRegistry;

use super::super::{TEST_PERMUTATION_QUEUE, select_duel_roles};

struct TestHost {
    data_root: PathBuf,
    scoreboard_dir: PathBuf,
    registry: ActivityExecutorRegistry,
    duel_model: Option<String>,
}

impl TestHost {
    fn new(duel_model: Option<&str>) -> Self {
        let temp_root = std::env::temp_dir().join("orbit-duel-role-test");
        Self {
            scoreboard_dir: temp_root.join("scoreboard"),
            data_root: temp_root,
            registry: ActivityExecutorRegistry::default(),
            duel_model: duel_model.map(ToOwned::to_owned),
        }
    }
}

impl RuntimeHost for TestHost {
    fn record_event(&self, _event: OrbitEvent) -> Result<(), OrbitError> {
        Ok(())
    }

    fn repo_root(&self) -> Result<String, OrbitError> {
        Ok(self.data_root.to_string_lossy().to_string())
    }

    fn data_root(&self) -> &Path {
        &self.data_root
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
        unimplemented!("not needed by duel role tests")
    }

    fn validate_activity_target_exists(
        &self,
        _target_type: JobTargetType,
        _target_id: &str,
    ) -> Result<Activity, OrbitError> {
        unimplemented!("not needed by duel role tests")
    }

    fn get_job(&self, _job_id: &str) -> Result<Option<Job>, OrbitError> {
        Ok(None)
    }

    fn invocation_records(
        &self,
        _query: orbit_store::InvocationQuery,
    ) -> Result<Vec<InvocationRecord>, OrbitError> {
        Ok(Vec::new())
    }

    fn run_tool_with_context_and_role(
        &self,
        _name: &str,
        _input: Value,
        _role: Role,
        _tool_context: ToolContext,
    ) -> Result<Value, OrbitError> {
        unimplemented!("not needed by duel role tests")
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

    fn resolved_agent_model_pair(&self, agent_cli: &str) -> Option<AgentModelPair> {
        match agent_cli {
            "codex" => Some(AgentModelPair::new("M_exec", "_")),
            "claude" => Some(AgentModelPair::new("opus-4.7", "sonnet-4.6")),
            "gemini" => Some(AgentModelPair::new("pro", "flash")),
            _ => None,
        }
    }

    fn duel_candidate_families(&self) -> Vec<String> {
        ["codex", "claude", "gemini"]
            .iter()
            .map(|family| (*family).to_string())
            .collect()
    }

    fn duel_orchestrator_model(&self, family: &str) -> Option<String> {
        if family == "codex" {
            self.duel_model.clone()
        } else {
            None
        }
    }

    fn scoring_enabled(&self) -> bool {
        false
    }

    fn graph_editing(&self) -> bool {
        false
    }

    fn scoreboard_dir(&self) -> &Path {
        &self.scoreboard_dir
    }
}

impl TaskReadHost for TestHost {
    fn get_task(&self, _task_id: &str) -> Result<Task, OrbitError> {
        unimplemented!("not needed by duel role tests")
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
        _external_ref: Option<&ExternalRef>,
        _has_external_ref_system: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        Ok(Vec::new())
    }
}

impl TaskWriteHost for TestHost {
    fn start_task(
        &self,
        _task_id: &str,
        _note: Option<String>,
        _comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        unimplemented!("not needed by duel role tests")
    }

    fn admit_task_for_workflow(&self, _task_id: &str, _workflow: &str) -> Result<Task, OrbitError> {
        unimplemented!("not needed by duel role tests")
    }

    fn update_task_from_activity(
        &self,
        _task_id: &str,
        _update: TaskActivityUpdate,
    ) -> Result<Task, OrbitError> {
        unimplemented!("not needed by duel role tests")
    }

    fn apply_task_automation_update(
        &self,
        _task_id: &str,
        _update: TaskAutomationUpdate,
    ) -> Result<(), OrbitError> {
        Ok(())
    }
}

fn queue_permutation(perm: [usize; 3]) {
    TEST_PERMUTATION_QUEUE.with(|cell| {
        let mut queue = cell.borrow_mut();
        queue.clear();
        queue.push_back(perm);
    });
}

#[test]
fn select_duel_roles_prefers_duel_model_then_resolved_pair() {
    queue_permutation([0, 1, 2]);
    let host = TestHost::new(Some("M_duel"));
    let output = select_duel_roles(&host, &json!({ "task_id": "ORB-TEST" }))
        .expect("role selection uses duel model");
    assert_eq!(output["duel_roles"]["implementer"]["model"], "M_duel");

    queue_permutation([0, 1, 2]);
    let host = TestHost::new(None);
    let output = select_duel_roles(&host, &json!({ "task_id": "ORB-TEST" }))
        .expect("role selection falls back to resolved pair");
    assert_eq!(output["duel_roles"]["implementer"]["model"], "M_exec");
}
