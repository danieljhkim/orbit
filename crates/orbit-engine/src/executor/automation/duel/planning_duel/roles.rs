use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Utc;
use orbit_common::types::{
    Activity, OrbitError, PlanningRoleAssignment, PlanningRoles, all_agent_families,
};
use serde_json::{Value, json};

use crate::context::RuntimeHost;

use crate::executor::automation::input::{input_string_field, required_input_string};

use super::super::{role_permutation_at, validate_role_permutation};

pub(super) const PLANNER_ACTIVITY_ID: &str = "propose_duel_plan";
pub(super) const ARBITER_ACTIVITY_ID: &str = "arbitrate_duel_plan";
pub(super) const PLANNER_TIMEOUT_SECONDS: u64 = 1800;
pub(super) const ARBITER_TIMEOUT_SECONDS: u64 = 900;

const PLANNING_DUEL_INSTRUCTION: &str = r#"Only use skills listed in this activity's skill_refs. Ignore all others.
You are a PLANNER in an Orbit planning duel. Inspect the task and surrounding
code, draft one implementation-ready proposal, and persist it to the task's
`artifacts/` directory. Do not edit source files, open PRs, or rely on your
structured response as the workflow handoff.

Steps:
1. Load the task:
   - Call orbit.task.show with input: {"id": "<task_id>"} to fetch the task title,
     description, plan, acceptance_criteria, context_files, and workspace_path.

2. Determine your artifact path from the active agent signature:
   - Your active agent family is `{{agent_family}}`.
   - Your active orchestrator model is `{{orchestrator_model}}`.
   - Your plan artifact path must be `planning-duel/{{agent_family}}-{{orchestrator_model}}.md`.
   - Do not invent a role-specific filename such as `planner_a.md` or `planner_b.md`.

3. Gather context with the graph surface first:
   - Build graph selectors from task.context_files and call orbit.graph.pack.
   - If pack returns knowledge_unavailable, use orbit.graph.overview to map the
     affected area, then orbit.graph.search, orbit.graph.refs, and orbit.graph.show
     to discover the relevant symbols and relationships.
   - If pack returns unresolved selectors, fall back to fs.read only for those paths.
   - Prefer orbit.graph.pack/search/show/overview/refs over raw file reads whenever
     the graph has the needed knowledge.

4. Draft exactly one proposal as markdown:
   - The first line must be exactly: `*authored by: {{agent_family}} / {{orchestrator_model}}*`
   - Then include these sections:
     ## Plan
     ## Context Files
     ## Risks
   - Keep the plan concise, implementation-ready, and specific to the current codebase.
   - Ignore any existing planner artifact for the other role. Your proposal must be independently reasoned.

5. Persist the proposal as a task artifact:
   - Use orbit.duel.plan.add to write the artifact under the signature-derived path.
   - Exact example:
     {"id":"<task_id>","content":"*authored by: {{agent_family}} / {{orchestrator_model}}*\n## Plan\n..."}

6. Stay narrowly scoped:
   - Do not edit source files, update task.plan, or touch PR state.
   - The only permitted mutation is writing your own planner artifact via orbit.duel.plan.add.

7. Structured output is optional:
   - The workflow does not depend on your response payload. Persist the artifact correctly even if you return null."#;
const ARBITER_INSTRUCTION: &str = r#"Only use skills listed in this activity's skill_refs. Ignore all others.
You are the ARBITER in an Orbit planning duel. Your job is to compare the
two submitted planner artifacts, choose the better one, and persist the
winning decision to the task artifact bundle.

Steps:
1. Load the task:
   - Call orbit.task.show with input: {"id": "<task_id>"} to fetch the task title,
     description, plan, acceptance_criteria, and context_files.

2. Load only the planner artifacts:
   - Call orbit.task.show with input: {"id":"<task_id>","field":"artifacts"} to fetch only the task artifacts.
   - From that response, inspect planner markdown artifacts under `planning-duel/` and ignore `planning-duel/winner.json`.
   - There must be exactly two planner markdown artifacts for this duel. If there are not exactly two, fail instead of guessing.
   - Treat both planner artifacts as read-only inputs. Do not invent a third plan.

3. Infer planner identity from the artifact signatures:
   - The first line of each planner artifact must be `*authored by: <agent> / <model>*`.
   - Parse those lines to recover each planner's agent CLI family and model.
   - The artifact signature is the canonical planner identity source.

4. Use the graph surface to verify claims:
   - Prefer orbit.graph.overview, orbit.graph.search, orbit.graph.refs, orbit.graph.show,
     and orbit.graph.pack for spot checks against the codebase.
   - Fall back to fs.read only when the graph does not have enough knowledge.

5. Decide the winner:
   - Choose the artifact proposal that is more feasible, complete, scoped, and aligned
     with the current codebase.
   - Keep a short `arbiter_rationale` that explains why the winning proposal is better.

6. Persist the winner marker:
   - Use orbit.duel.plan.winner to write `planning-duel/winner.json`.
   - Exact example:
     {"id":"<task_id>","winner_agent_cli":"codex","winner_model":"gpt-5.4","arbiter_rationale":"More concrete writeback and test coverage."}

7. Stay narrowly scoped:
   - Do not edit source files, update task.plan directly, or open PRs.
   - The only permitted mutation is writing `planning-duel/winner.json` via orbit.duel.plan.winner."#;

thread_local! {
    static TEST_PERMUTATION_QUEUE: RefCell<VecDeque<[usize; 3]>> =
        const { RefCell::new(VecDeque::new()) };
}

fn next_permutation() -> Result<[usize; 3], OrbitError> {
    let family_count = all_agent_families().len();
    let from_test = TEST_PERMUTATION_QUEUE.with(|cell| cell.borrow_mut().pop_front());
    if let Some(perm) = from_test {
        return validate_role_permutation(perm, family_count, "select_planning_duel_roles");
    }

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    role_permutation_at(family_count, nanos as usize)
}

fn orchestrator_model_for<H: RuntimeHost + ?Sized>(
    host: &H,
    family: &str,
) -> Result<String, OrbitError> {
    host.resolved_agent_model_pair(family)
        .map(|pair| pair.orchestrator)
        .ok_or_else(|| {
            OrbitError::Execution(format!(
                "no registered model pair for agent family '{family}'"
            ))
        })
}

fn build_role_assignment<H: RuntimeHost + ?Sized>(
    host: &H,
    family: &str,
) -> Result<PlanningRoleAssignment, OrbitError> {
    Ok(PlanningRoleAssignment {
        agent: family.to_string(),
        model: orchestrator_model_for(host, family)?,
    })
}

fn build_roles_output<H: RuntimeHost + ?Sized>(
    host: &H,
    perm: [usize; 3],
) -> Result<Value, OrbitError> {
    let families = all_agent_families();
    let perm = validate_role_permutation(perm, families.len(), "select_planning_duel_roles")?;
    let planner_a = families[perm[0]];
    let planner_b = families[perm[1]];
    let arbiter = families[perm[2]];

    let started_at = Utc::now().to_rfc3339();

    Ok(json!({
        "planner_a_agent_cli": planner_a,
        "planner_a_model": orchestrator_model_for(host, planner_a)?,
        "planner_b_agent_cli": planner_b,
        "planner_b_model": orchestrator_model_for(host, planner_b)?,
        "arbiter_agent_cli": arbiter,
        "arbiter_model": orchestrator_model_for(host, arbiter)?,
        "planning_duel_started_at": started_at,
        "planning_duel_roles": {
            "planner_a": build_role_assignment(host, planner_a)?,
            "planner_b": build_role_assignment(host, planner_b)?,
            "arbiter": build_role_assignment(host, arbiter)?,
        }
    }))
}

fn planning_duel_agent_activity(
    id: &str,
    description: &str,
    instruction: &str,
    tools: &[&str],
) -> Activity {
    let now = Utc::now();
    Activity {
        id: id.to_string(),
        spec_type: "agent_invoke".to_string(),
        description: description.to_string(),
        input_schema_json: json!({
            "type": "object",
            "required": ["task_id"],
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Orbit task ID for the planning duel."
                }
            }
        }),
        output_schema_json: json!({}),
        spec_config: json!({
            "instruction": instruction,
            "skill_refs": ["orbit", "orbit-graph"],
        }),
        tools: tools.iter().map(|tool| (*tool).to_string()).collect(),
        proc_allowed_programs: Vec::new(),
        executor: None,
        workspace_path: None,
        created_by: Some("system".to_string()),
        is_active: true,
        created_at: now,
        updated_at: now,
    }
}

pub(super) fn planner_activity() -> Activity {
    planning_duel_agent_activity(
        PLANNER_ACTIVITY_ID,
        "Draft one planning-duel proposal, then persist it as a task artifact using the graph surface.",
        PLANNING_DUEL_INSTRUCTION,
        &[
            "orbit.task.show",
            "orbit.duel.plan.add",
            "orbit.graph.pack",
            "orbit.graph.search",
            "orbit.graph.show",
            "orbit.graph.overview",
            "orbit.graph.refs",
            "fs.read",
        ],
    )
}

pub(super) fn arbiter_activity() -> Activity {
    planning_duel_agent_activity(
        ARBITER_ACTIVITY_ID,
        "Choose the better of two planning-duel task artifacts for a single task and persist the winner marker.",
        ARBITER_INSTRUCTION,
        &[
            "orbit.task.show",
            "orbit.duel.plan.winner",
            "orbit.graph.pack",
            "orbit.graph.search",
            "orbit.graph.show",
            "orbit.graph.overview",
            "orbit.graph.refs",
            "fs.read",
        ],
    )
}

pub(super) fn planner_input(task_id: &str) -> Value {
    json!({ "task_id": task_id })
}

pub(super) fn arbiter_input(task_id: &str) -> Value {
    json!({ "task_id": task_id })
}

pub(super) fn parse_planning_duel_roles(input: &Value) -> Result<PlanningRoles, OrbitError> {
    serde_json::from_value(input.get("planning_duel_roles").cloned().ok_or_else(|| {
        OrbitError::InvalidInput("missing required input.planning_duel_roles".to_string())
    })?)
    .map_err(|err| OrbitError::InvalidInput(format!("invalid planning_duel_roles payload: {err}")))
}

#[allow(dead_code)]
pub(super) fn role_activity_id(input: &Value, role: &str) -> Result<String, OrbitError> {
    let flat_key = format!("{role}_activity_id");
    if let Some(value) = input_string_field(input, &flat_key) {
        return Ok(value);
    }
    if matches!(role, "planner_a" | "planner_b")
        && let Some(value) = input_string_field(input, "planner_activity_id")
    {
        return Ok(value);
    }

    input
        .get("roles")
        .and_then(|roles| roles.get(role))
        .and_then(|entry| {
            entry
                .get("activity_id")
                .and_then(Value::as_str)
                .or_else(|| entry.get("activityId").and_then(Value::as_str))
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| OrbitError::InvalidInput(format!("missing required input.{flat_key}")))
}

pub(super) fn select_planning_duel_roles<H: RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let perm = next_permutation()?;
    let output = build_roles_output(host, perm)?;

    Ok(json!({
        "task_id": task_id,
        "planning_duel_started_at": output["planning_duel_started_at"].clone(),
        "planner_a_agent_cli": output["planner_a_agent_cli"].clone(),
        "planner_a_model": output["planner_a_model"].clone(),
        "planner_b_agent_cli": output["planner_b_agent_cli"].clone(),
        "planner_b_model": output["planner_b_model"].clone(),
        "arbiter_agent_cli": output["arbiter_agent_cli"].clone(),
        "arbiter_model": output["arbiter_model"].clone(),
        "planning_duel_roles": output["planning_duel_roles"].clone(),
    }))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use orbit_common::types::{Activity, AgentModelPair, Job, JobTargetType, OrbitEvent, Role};
    use orbit_store::InvocationRecord;
    use orbit_tools::ToolContext;

    use crate::context::{JobRunResult, RuntimeHost};
    use crate::executor::registry::ActivityExecutorRegistry;

    use super::*;

    struct TestHost {
        data_root: PathBuf,
        scoreboard_dir: PathBuf,
        registry: ActivityExecutorRegistry,
    }

    impl TestHost {
        fn new() -> Self {
            let temp_root = std::env::temp_dir().join("orbit-planning-duel-role-test");
            Self {
                scoreboard_dir: temp_root.join("scoreboard"),
                data_root: temp_root,
                registry: ActivityExecutorRegistry::default(),
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
                "codex" => Some(AgentModelPair::new("gpt-5.5", "gpt-5.4-mini")),
                "claude" => Some(AgentModelPair::new("opus-4.7", "sonnet-4.6")),
                "gemini" => Some(AgentModelPair::new("pro", "flash")),
                "grok" => Some(AgentModelPair::new("grok-4", "grok-3")),
                _ => None,
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

    #[test]
    fn planning_duel_role_output_can_assign_grok() {
        let host = TestHost::new();
        let output = build_roles_output(&host, [3, 0, 1]).expect("roles output");

        assert_eq!(output["planner_a_agent_cli"], "grok");
        assert_eq!(output["planner_a_model"], "grok-4");
        assert_eq!(output["planning_duel_roles"]["planner_a"]["agent"], "grok");
        assert_eq!(
            output["planning_duel_roles"]["planner_a"]["model"],
            "grok-4"
        );
        assert_eq!(output["planner_b_agent_cli"], "codex");
        assert_eq!(output["arbiter_agent_cli"], "claude");
    }
}
