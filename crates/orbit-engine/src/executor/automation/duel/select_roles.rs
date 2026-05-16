//! `select_duel_roles` automation.
//!
//! Generates a random ordered selection of three distinct agent families
//! from Orbit's current family set across the three duel roles
//! (implementer, reviewer, arbiter) and writes them into the current
//! job input so downstream steps can resolve per-role agent CLIs via
//! `agent_cli_from_input` / `model_from_input`.
//!
//! The implementer is also stamped onto the task's internal routing
//! fields so that the reusable `implement_change` activity — which
//! resolves the agent from task metadata — picks up the implementer for
//! this run.
//!
//! Randomness source: `SystemTime` nanoseconds modulo the possible
//! permutations. A thread-local test seam lets unit tests inject a
//! deterministic queue of permutation indices, so behavior can be
//! verified without patching the clock.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Utc;
use orbit_common::types::OrbitError;
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

use super::super::input::required_input_string;
use super::{role_permutation_at, validate_role_permutation};

thread_local! {
    /// Test seam. When non-empty, each call to the executor pops the
    /// front entry and uses it as the permutation instead of consulting
    /// the clock.
    static TEST_PERMUTATION_QUEUE: RefCell<VecDeque<[usize; 3]>> =
        const { RefCell::new(VecDeque::new()) };
}

/// Pick the next permutation of family indices — test override first,
/// otherwise derive from `SystemTime` nanoseconds.
fn next_permutation<H: RuntimeHost + ?Sized>(host: &H) -> Result<[usize; 3], OrbitError> {
    let family_count = host.duel_candidate_families().len();
    let from_test = TEST_PERMUTATION_QUEUE.with(|cell| cell.borrow_mut().pop_front());
    if let Some(perm) = from_test {
        return validate_role_permutation(perm, family_count, "select_duel_roles");
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    role_permutation_at(family_count, nanos as usize)
}

/// Shape of the JSON object this executor writes into current_input.
///
/// Downstream steps resolve their agent via `agent_cli_from_input`
/// (referencing one of the `*_agent_cli` fields) and their model via
/// `model_from_input` (referencing the matching `*_model` field).
fn build_roles_output<H: RuntimeHost + ?Sized>(
    host: &H,
    perm: [usize; 3],
) -> Result<Value, OrbitError> {
    let families = host.duel_candidate_families();
    let perm = validate_role_permutation(perm, families.len(), "select_duel_roles")?;
    let implementer = families[perm[0]].as_str();
    let reviewer = families[perm[1]].as_str();
    let arbiter = families[perm[2]].as_str();

    let implementer_model = orchestrator_model_for(host, implementer)?;
    let reviewer_model = orchestrator_model_for(host, reviewer)?;
    let arbiter_model = orchestrator_model_for(host, arbiter)?;

    // Stamp wall-clock start time so `record_duel_scores` — which runs at
    // the very end of the pipeline — can compute `cost.wall_clock_seconds`
    // without reaching into job runner internals. The timestamp flows
    // through current_input automatically.
    let started_at = Utc::now().to_rfc3339();

    Ok(json!({
        "implementer_agent_cli": implementer,
        "implementer_model": implementer_model,
        "reviewer_agent_cli": reviewer,
        "reviewer_model": reviewer_model,
        "arbiter_agent_cli": arbiter,
        "arbiter_model": arbiter_model,
        "duel_started_at": started_at,
        "duel_roles": {
            "implementer": { "agent": implementer, "model": implementer_model },
            "reviewer":    { "agent": reviewer,    "model": reviewer_model },
            "arbiter":     { "agent": arbiter,     "model": arbiter_model },
        }
    }))
}

fn orchestrator_model_for<H: RuntimeHost + ?Sized>(
    host: &H,
    family: &str,
) -> Result<String, OrbitError> {
    if let Some(model) = host.duel_orchestrator_model(family) {
        return Ok(model);
    }
    host.resolved_agent_model_pair(family)
        .map(|pair| pair.orchestrator)
        .ok_or_else(|| {
            OrbitError::Execution(format!(
                "no registered model pair for agent family '{family}' — \
                 update orbit_common::types::agent_pair::resolve_agent_model_pair"
            ))
        })
}

pub(in crate::executor::automation) fn select_duel_roles<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;

    let perm = next_permutation(host)?;
    let output = build_roles_output(host, perm)?;

    // Stamp the implementer onto the task's actor identity so that the
    // shared `implement_change` activity (which falls back to the task
    // actor when `step.agent_cli` is empty) uses the duel implementer.
    let implementer_agent = output
        .get("implementer_agent_cli")
        .and_then(Value::as_str)
        .map(str::to_string);
    let implementer_model = output
        .get("implementer_model")
        .and_then(Value::as_str)
        .map(str::to_string);

    host.apply_task_automation_update(
        task_id,
        TaskAutomationUpdate {
            agent: implementer_agent,
            model: implementer_model,
            ..TaskAutomationUpdate::default()
        },
    )?;

    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use orbit_common::types::{
        Activity, AgentModelPair, ExternalRef, Job, JobTargetType, OrbitEvent, Role, Task,
        TaskArtifact, TaskPriority, TaskStatus,
    };
    use orbit_store::InvocationRecord;
    use orbit_tools::ToolContext;

    use crate::context::{
        JobRunResult, RuntimeHost, TaskAutomationUpdate, TaskReadHost, TaskWriteHost,
    };
    use crate::executor::registry::ActivityExecutorRegistry;

    use super::*;

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

        fn admit_task_for_workflow(
            &self,
            _task_id: &str,
            _workflow: &str,
        ) -> Result<Task, OrbitError> {
            unimplemented!("not needed by duel role tests")
        }

        fn update_task_from_activity(
            &self,
            _task_id: &str,
            _status: TaskStatus,
            _execution_summary: Option<String>,
            _comment: Option<String>,
            _note: Option<String>,
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
}
