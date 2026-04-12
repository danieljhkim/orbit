//! `select_duel_roles` automation.
//!
//! Generates a random permutation of the three agent families
//! (`codex`, `claude`, `gemini`) across the three duel roles
//! (implementer, reviewer, arbiter) and writes them into the current
//! job input so downstream steps can resolve per-role agent CLIs via
//! `agent_cli_from_input` / `model_from_input`.
//!
//! The implementer is also stamped onto the task's `actor_identity` so
//! that the reusable `implement_change` activity — which resolves the
//! agent from the task actor — picks up the implementer for this run.
//!
//! Randomness source: `SystemTime` nanoseconds modulo the six possible
//! permutations. A thread-local test seam lets unit tests inject a
//! deterministic queue of permutation indices, so behavior can be
//! verified without patching the clock.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Utc;
use orbit_types::{OrbitError, all_agent_families};
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

use super::input::required_input_string;

/// The six permutations of `[0, 1, 2]`, used to assign families to
/// `(implementer, reviewer, arbiter)` slots.
const PERMUTATIONS: [[usize; 3]; 6] = [
    [0, 1, 2],
    [0, 2, 1],
    [1, 0, 2],
    [1, 2, 0],
    [2, 0, 1],
    [2, 1, 0],
];

thread_local! {
    /// Test seam. When non-empty, each call to the executor pops the
    /// front entry and uses it as the permutation instead of consulting
    /// the clock.  Populated via [`push_test_permutations`].
    static TEST_PERMUTATION_QUEUE: RefCell<VecDeque<[usize; 3]>> =
        const { RefCell::new(VecDeque::new()) };
}

/// Seed the thread-local test permutation queue. Each subsequent call
/// to `select_duel_roles` on this thread consumes one entry.
#[cfg(test)]
pub(crate) fn push_test_permutations(perms: impl IntoIterator<Item = [usize; 3]>) {
    TEST_PERMUTATION_QUEUE.with(|cell| {
        cell.borrow_mut().extend(perms);
    });
}

#[cfg(test)]
pub(crate) fn clear_test_permutations() {
    TEST_PERMUTATION_QUEUE.with(|cell| cell.borrow_mut().clear());
}

/// Pick the next permutation of family indices — test override first,
/// otherwise derive from `SystemTime` nanoseconds.
fn next_permutation() -> [usize; 3] {
    let from_test = TEST_PERMUTATION_QUEUE.with(|cell| cell.borrow_mut().pop_front());
    if let Some(perm) = from_test {
        return perm;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    PERMUTATIONS[(nanos as usize) % PERMUTATIONS.len()]
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
    let families = all_agent_families();
    let implementer = families[perm[0]];
    let reviewer = families[perm[1]];
    let arbiter = families[perm[2]];

    // Sanity check — the permutation generator must produce distinct
    // indices. A failure here means PERMUTATIONS was corrupted.
    if implementer == reviewer || reviewer == arbiter || implementer == arbiter {
        return Err(OrbitError::Execution(format!(
            "select_duel_roles produced non-distinct families: \
             implementer={implementer}, reviewer={reviewer}, arbiter={arbiter}"
        )));
    }

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
    host.resolved_agent_model_pair(family)
        .map(|pair| pair.orchestrator)
        .ok_or_else(|| {
            OrbitError::Execution(format!(
                "no registered model pair for agent family '{family}' — \
                 update orbit_types::agent_pair::resolve_agent_model_pair"
            ))
        })
}

pub(super) fn select_duel_roles<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;

    let perm = next_permutation();
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
    use super::*;
    use orbit_store::{InvocationQuery, InvocationRecord};
    use orbit_tools::ToolContext;
    use orbit_types::{Activity, Job, JobTargetType, OrbitEvent, Role};
    use orbit_types::{Task, TaskPriority, TaskStatus};
    use std::collections::BTreeSet;
    use std::path::Path;
    use std::sync::Mutex;

    #[derive(Default)]
    struct StubTaskHost {
        updates: Mutex<Vec<(String, TaskAutomationUpdate)>>,
    }

    impl TaskHost for StubTaskHost {
        fn get_task(&self, _task_id: &str) -> Result<Task, OrbitError> {
            unimplemented!("not used by select_duel_roles")
        }
        fn get_task_artifacts(
            &self,
            _task_id: &str,
        ) -> Result<Vec<orbit_types::TaskArtifact>, OrbitError> {
            Ok(Vec::new())
        }
        fn list_tasks_filtered(
            &self,
            _status: Option<TaskStatus>,
            _priority: Option<TaskPriority>,
            _parent_id: Option<&str>,
            _batch_id: Option<&str>,
        ) -> Result<Vec<Task>, OrbitError> {
            Ok(Vec::new())
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
            task_id: &str,
            update: TaskAutomationUpdate,
        ) -> Result<(), OrbitError> {
            self.updates
                .lock()
                .unwrap()
                .push((task_id.to_string(), update));
            Ok(())
        }
    }

    impl RuntimeHost for StubTaskHost {
        fn record_event(&self, _event: OrbitEvent) -> Result<(), OrbitError> {
            unimplemented!("not used by select_duel_roles")
        }

        fn repo_root(&self) -> Result<String, OrbitError> {
            unimplemented!("not used by select_duel_roles")
        }

        fn data_root(&self) -> &Path {
            Path::new(".")
        }

        fn run_job_now_with_input_debug(
            &self,
            _job_id: &str,
            _input: Value,
            _debug: bool,
        ) -> Result<crate::JobRunResult, OrbitError> {
            unimplemented!("not used by select_duel_roles")
        }

        fn validate_activity_target_exists(
            &self,
            _target_type: JobTargetType,
            _target_id: &str,
        ) -> Result<Activity, OrbitError> {
            unimplemented!("not used by select_duel_roles")
        }

        fn get_job(&self, _job_id: &str) -> Result<Option<Job>, OrbitError> {
            unimplemented!("not used by select_duel_roles")
        }

        fn invocation_records(
            &self,
            _query: InvocationQuery,
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
            unimplemented!("not used by select_duel_roles")
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
            Path::new(".")
        }
    }

    fn role_triple(value: &Value) -> (String, String, String) {
        (
            value["implementer_agent_cli"].as_str().unwrap().to_string(),
            value["reviewer_agent_cli"].as_str().unwrap().to_string(),
            value["arbiter_agent_cli"].as_str().unwrap().to_string(),
        )
    }

    #[test]
    fn three_deterministic_runs_produce_distinct_pairwise_role_assignments() {
        clear_test_permutations();
        push_test_permutations([[0, 1, 2], [1, 2, 0], [2, 0, 1]]);

        let host = StubTaskHost::default();
        let input = json!({ "task_id": "T-duel-1" });

        let expected: &[(&str, &str, &str)] = &[
            ("codex", "claude", "gemini"),
            ("claude", "gemini", "codex"),
            ("gemini", "codex", "claude"),
        ];

        for (i, exp) in expected.iter().enumerate() {
            let out = select_duel_roles(&host, &input).expect("select_duel_roles ok");

            let (implementer, reviewer, arbiter) = role_triple(&out);

            // All three roles drawn from the canonical candidate set.
            let as_set: BTreeSet<String> = [&implementer, &reviewer, &arbiter]
                .iter()
                .map(|s| s.to_string())
                .collect();
            let canonical: BTreeSet<String> = all_agent_families()
                .into_iter()
                .map(str::to_string)
                .collect();
            assert_eq!(as_set, canonical, "run {i} must use full family set");

            // Pairwise distinct.
            assert_ne!(implementer, reviewer, "run {i} impl==reviewer");
            assert_ne!(reviewer, arbiter, "run {i} reviewer==arbiter");
            assert_ne!(implementer, arbiter, "run {i} impl==arbiter");

            // Matches the permutation we seeded.
            assert_eq!(implementer, exp.0, "run {i} implementer");
            assert_eq!(reviewer, exp.1, "run {i} reviewer");
            assert_eq!(arbiter, exp.2, "run {i} arbiter");
        }
    }

    #[test]
    fn stamps_implementer_onto_task_actor_identity() {
        clear_test_permutations();
        push_test_permutations([[1, 0, 2]]);

        let host = StubTaskHost::default();
        let input = json!({ "task_id": "T-duel-stamp" });

        let out = select_duel_roles(&host, &input).expect("ok");
        assert_eq!(out["implementer_agent_cli"], json!("claude"));

        let updates = host.updates.lock().unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].0, "T-duel-stamp");
        assert_eq!(updates[0].1.agent.as_deref(), Some("claude"));
        assert_eq!(updates[0].1.model.as_deref(), Some("opus-4.6"));
    }

    #[test]
    fn missing_task_id_returns_invalid_input() {
        clear_test_permutations();
        let host = StubTaskHost::default();
        let err = select_duel_roles(&host, &json!({})).unwrap_err();
        assert!(matches!(err, OrbitError::InvalidInput(_)));
    }

    #[test]
    fn duel_roles_subobject_carries_structured_agent_model_pairs() {
        clear_test_permutations();
        push_test_permutations([[0, 1, 2]]);

        let host = StubTaskHost::default();
        let out = select_duel_roles(&host, &json!({ "task_id": "T-struct" })).unwrap();

        let roles = out.get("duel_roles").expect("duel_roles key");
        assert_eq!(roles["implementer"]["agent"], json!("codex"));
        assert_eq!(roles["implementer"]["model"], json!("gpt-5.4"));
        assert_eq!(roles["reviewer"]["agent"], json!("claude"));
        assert_eq!(roles["reviewer"]["model"], json!("opus-4.6"));
        assert_eq!(roles["arbiter"]["agent"], json!("gemini"));
        assert_eq!(roles["arbiter"]["model"], json!("gemini-3.1-pro-preview"));
    }

    #[test]
    fn output_stamps_rfc3339_duel_started_at_for_wall_clock_cost() {
        clear_test_permutations();
        push_test_permutations([[0, 1, 2]]);
        let host = StubTaskHost::default();
        let out = select_duel_roles(&host, &json!({ "task_id": "T-ts" })).unwrap();
        let ts = out["duel_started_at"]
            .as_str()
            .expect("duel_started_at string");
        chrono::DateTime::parse_from_rfc3339(ts).expect("duel_started_at must be RFC3339");
    }

    #[test]
    fn time_seeded_permutation_without_test_queue_is_valid() {
        clear_test_permutations();
        let host = StubTaskHost::default();
        let out = select_duel_roles(&host, &json!({ "task_id": "T-time" })).unwrap();
        let (i, r, a) = role_triple(&out);
        let set: BTreeSet<String> = [i, r, a].into_iter().collect();
        assert_eq!(set.len(), 3, "three pairwise-distinct families");
        for family in &set {
            assert!(
                all_agent_families().contains(&family.as_str()),
                "unexpected family {family}"
            );
        }
    }
}
