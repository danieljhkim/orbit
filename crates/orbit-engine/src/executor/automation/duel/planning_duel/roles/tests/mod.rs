#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use orbit_common::types::{
    Activity, AgentFamily, AgentModelPair, Job, JobTargetType, OrbitError, OrbitEvent,
    PlanningRoles, Role, RoleSlot, TaskArtifact,
};
use orbit_store::InvocationRecord;
use orbit_tools::ToolContext;
use serde_json::{Value, json};

use crate::context::{JobRunResult, RuntimeHost};
use crate::executor::registry::ActivityExecutorRegistry;

use super::select_planning_duel_roles; // for tests that call it; build_roles_output will be pub(crate)

struct TestHost {
    data_root: PathBuf,
    scoreboard_dir: PathBuf,
    registry: ActivityExecutorRegistry,
    duel_models: BTreeMap<String, String>,
}

impl TestHost {
    fn new() -> Self {
        Self::with_duel_model(None)
    }

    fn with_duel_model(duel_model: Option<&str>) -> Self {
        let mut duel_models = BTreeMap::new();
        if let Some(duel_model) = duel_model {
            duel_models.insert("codex".to_string(), duel_model.to_string());
        }
        Self::with_duel_models(duel_models)
    }

    fn with_family_duel_model(family: &str, model: &str) -> Self {
        let mut duel_models = BTreeMap::new();
        duel_models.insert(family.to_string(), model.to_string());
        Self::with_duel_models(duel_models)
    }

    fn with_duel_models(duel_models: BTreeMap<String, String>) -> Self {
        let temp_root = std::env::temp_dir().join("orbit-planning-duel-role-test");
        Self {
            scoreboard_dir: temp_root.join("scoreboard"),
            data_root: temp_root,
            registry: ActivityExecutorRegistry::default(),
            duel_models,
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
        unimplemented!("not needed by planning-duel role tests")
    }

    fn validate_activity_target_exists(
        &self,
        _target_type: JobTargetType,
        _target_id: &str,
    ) -> Result<Activity, OrbitError> {
        unimplemented!("not needed by planning-duel role tests")
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
        unimplemented!("not needed by planning-duel role tests")
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
            "grok" => Some(AgentModelPair::new("grok-4", "grok-3")),
            _ => None,
        }
    }

    fn duel_orchestrator_model(&self, family: &str) -> Option<String> {
        self.duel_models.get(family).cloned()
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

fn queue_permutation(perm: [usize; 3]) {
    super::TEST_PERMUTATION_QUEUE.with(|cell| {
        let mut queue = cell.borrow_mut();
        queue.clear();
        queue.push_back(perm);
    });
}

mod assignment;
mod selection;
