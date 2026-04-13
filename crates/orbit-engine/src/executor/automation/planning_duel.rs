//! Planning-duel automation helpers.
//!
//! The unified planning-duel automation:
//! - selects two distinct planners plus one arbiter
//! - runs the two planners concurrently via real agent CLIs
//! - runs the arbiter after both planner artifacts exist
//! - writes back the winning plan and appends a planning-duel scoreboard record

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Utc;
use orbit_store::{InvocationRecord, planning_duel_scoreboard};
use orbit_tools::ToolContext;
use orbit_types::{
    Activity, EfficiencyMetrics, OrbitError, PlannerSlot, PlanningDuelRun, PlanningEfficiency,
    PlanningOutcome, PlanningRoleAssignment, PlanningRoles, Role, TaskArtifact, TaskComment,
    TokenUsage, all_agent_families,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::context::{ActivityInvocationResult, RuntimeHost, TaskAutomationUpdate, TaskHost};

use super::input::{input_string_field, required_input_string};

const PERMUTATIONS: [[usize; 3]; 6] = [
    [0, 1, 2],
    [0, 2, 1],
    [1, 0, 2],
    [1, 2, 0],
    [2, 0, 1],
    [2, 1, 0],
];
const PLANNING_DUEL_ARTIFACT_PREFIX: &str = "planning-duel/";
const PLANNING_DUEL_PLAN_EXTENSION: &str = ".md";
const WINNER_ARTIFACT_PATH: &str = "planning-duel/winner.json";
const TASKS_DIR_NAME: &str = "tasks";
const TASK_ARTIFACTS_DIR_NAME: &str = "artifacts";
const AUTHOR_SIGNATURE_PREFIX: &str = "*authored by: ";
const AUTHOR_SIGNATURE_SEPARATOR: &str = " / ";
const PLANNER_ACTIVITY_ID: &str = "propose_duel_plan";
const ARBITER_ACTIVITY_ID: &str = "arbitrate_duel_plan";
const PLANNER_TIMEOUT_SECONDS: u64 = 1800;
const ARBITER_TIMEOUT_SECONDS: u64 = 900;
const PLANNER_INSTRUCTION: &str = r#"Only use skills listed in this activity's skill_refs. Ignore all others.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanningDuelRoleMetrics {
    agent: String,
    model: String,
    activity_id: String,
    efficiency: PlanningDuelEfficiency,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct PlanningDuelEfficiency {
    invocation_count: u64,
    wall_clock_ms: u64,
    tool_call_count: u64,
    input_tokens: u64,
    cache_read_tokens: u64,
    cache_create_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    byte_proxy_total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PlanningDuelWinnerArtifact {
    winner_agent_cli: String,
    winner_model: String,
    artifact_path: String,
    arbiter_agent_cli: String,
    arbiter_model: String,
    arbiter_rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlanningDuelPlanArtifact {
    path: String,
    content: String,
    author: PlanningRoleAssignment,
}

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
    let planner_a = families[perm[0]];
    let planner_b = families[perm[1]];
    let arbiter = families[perm[2]];

    if planner_a == planner_b || planner_b == arbiter || planner_a == arbiter {
        return Err(OrbitError::Execution(format!(
            "select_planning_duel_roles produced non-distinct families: \
             planner_a={planner_a}, planner_b={planner_b}, arbiter={arbiter}"
        )));
    }

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
        workspace_path: None,
        created_by: Some("system".to_string()),
        is_active: true,
        created_at: now,
        updated_at: now,
    }
}

fn planner_activity() -> Activity {
    planning_duel_agent_activity(
        PLANNER_ACTIVITY_ID,
        "Draft one planning-duel proposal, then persist it as a task artifact using the graph surface.",
        PLANNER_INSTRUCTION,
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

fn arbiter_activity() -> Activity {
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

fn planner_input(task_id: &str) -> Value {
    json!({ "task_id": task_id })
}

fn arbiter_input(task_id: &str) -> Value {
    json!({ "task_id": task_id })
}

fn efficiency_from_invocation(invocation: &ActivityInvocationResult) -> PlanningDuelEfficiency {
    let trace = &invocation.invocation_trace;
    PlanningDuelEfficiency {
        invocation_count: 1,
        // The direct activity runner already records the authoritative elapsed
        // wall clock separately from the invocation trace payload.
        wall_clock_ms: invocation.duration_ms,
        tool_call_count: trace.tool_calls.len() as u64,
        input_tokens: trace.usage.input,
        cache_read_tokens: trace.usage.cache_read,
        cache_create_tokens: trace.usage.cache_create,
        output_tokens: trace.usage.output,
        total_tokens: trace
            .usage
            .input
            .saturating_add(trace.usage.cache_read)
            .saturating_add(trace.usage.cache_create)
            .saturating_add(trace.usage.output),
        byte_proxy_total: trace.tool_calls.iter().map(|call| call.result_bytes).sum(),
    }
}

fn role_metrics_from_invocation(
    role: &PlanningRoleAssignment,
    activity_id: &str,
    invocation: &ActivityInvocationResult,
) -> PlanningDuelRoleMetrics {
    PlanningDuelRoleMetrics {
        agent: role.agent.clone(),
        model: role.model.clone(),
        activity_id: activity_id.to_string(),
        efficiency: efficiency_from_invocation(invocation),
    }
}

fn join_activity_result(
    result: std::thread::Result<Result<ActivityInvocationResult, OrbitError>>,
    label: &str,
) -> Result<ActivityInvocationResult, OrbitError> {
    match result {
        Ok(inner) => inner,
        Err(_) => Err(OrbitError::Execution(format!(
            "{label} activity thread panicked"
        ))),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn role_activity_id(input: &Value, role: &str) -> Result<String, OrbitError> {
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

fn parse_planning_duel_roles(input: &Value) -> Result<PlanningRoles, OrbitError> {
    serde_json::from_value(input.get("planning_duel_roles").cloned().ok_or_else(|| {
        OrbitError::InvalidInput("missing required input.planning_duel_roles".to_string())
    })?)
    .map_err(|err| OrbitError::InvalidInput(format!("invalid planning_duel_roles payload: {err}")))
}

fn parse_planning_duel_signature(content: &str) -> Result<PlanningRoleAssignment, OrbitError> {
    let first_line = content
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .ok_or_else(|| {
            OrbitError::InvalidInput(
                "planning duel artifact must start with an authored-by signature line".to_string(),
            )
        })?;
    let signature = first_line
        .strip_prefix(AUTHOR_SIGNATURE_PREFIX)
        .and_then(|value| value.strip_suffix('*'))
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "planning duel artifact signature must match `{AUTHOR_SIGNATURE_PREFIX}<agent> / <model>*`"
            ))
        })?;
    let (agent, model) = signature
        .split_once(AUTHOR_SIGNATURE_SEPARATOR)
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "planning duel artifact signature must contain `{AUTHOR_SIGNATURE_SEPARATOR}`"
            ))
        })?;
    if agent.trim().is_empty() || model.trim().is_empty() {
        return Err(OrbitError::InvalidInput(
            "planning duel artifact signature must include both agent and model".to_string(),
        ));
    }
    Ok(PlanningRoleAssignment {
        agent: agent.trim().to_string(),
        model: model.trim().to_string(),
    })
}

fn planning_duel_plan_artifacts(
    artifacts: &[TaskArtifact],
) -> Result<Vec<PlanningDuelPlanArtifact>, OrbitError> {
    let mut plan_artifacts = artifacts
        .iter()
        .filter(|artifact| {
            artifact.path.starts_with(PLANNING_DUEL_ARTIFACT_PREFIX)
                && artifact.path.ends_with(PLANNING_DUEL_PLAN_EXTENSION)
        })
        .map(|artifact| {
            Ok(PlanningDuelPlanArtifact {
                path: artifact.path.clone(),
                content: artifact.content.clone(),
                author: parse_planning_duel_signature(&artifact.content)?,
            })
        })
        .collect::<Result<Vec<_>, OrbitError>>()?;
    plan_artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    if plan_artifacts.is_empty() {
        return Err(OrbitError::InvalidInput(
            "missing planning duel markdown artifacts under planning-duel/".to_string(),
        ));
    }
    Ok(plan_artifacts)
}

fn plan_artifact_for_assignment<'a>(
    plan_artifacts: &'a [PlanningDuelPlanArtifact],
    assignment: &PlanningRoleAssignment,
) -> Result<&'a PlanningDuelPlanArtifact, OrbitError> {
    let matches = plan_artifacts
        .iter()
        .filter(|artifact| artifact.author == *assignment)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [artifact] => Ok(*artifact),
        [] => Err(OrbitError::InvalidInput(format!(
            "missing planning duel artifact for {}/{}",
            assignment.agent, assignment.model
        ))),
        _ => Err(OrbitError::InvalidInput(format!(
            "found multiple planning duel artifacts for {}/{}",
            assignment.agent, assignment.model
        ))),
    }
}

fn plan_artifact_by_path<'a>(
    plan_artifacts: &'a [PlanningDuelPlanArtifact],
    artifact_path: &str,
) -> Result<&'a PlanningDuelPlanArtifact, OrbitError> {
    let matches = plan_artifacts
        .iter()
        .filter(|artifact| artifact.path == artifact_path)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [artifact] => Ok(*artifact),
        [] => Err(OrbitError::InvalidInput(format!(
            "missing planning duel artifact `{artifact_path}`"
        ))),
        _ => Err(OrbitError::InvalidInput(format!(
            "found multiple planning duel artifacts at `{artifact_path}`"
        ))),
    }
}

fn winner_artifact_from_artifacts(
    artifacts: &[TaskArtifact],
) -> Result<PlanningDuelWinnerArtifact, OrbitError> {
    let winner_artifact = artifacts
        .iter()
        .find(|artifact| artifact.path == WINNER_ARTIFACT_PATH)
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "missing required task artifact `{WINNER_ARTIFACT_PATH}`"
            ))
        })?;
    serde_json::from_str::<PlanningDuelWinnerArtifact>(&winner_artifact.content).map_err(|err| {
        OrbitError::InvalidInput(format!("invalid `{WINNER_ARTIFACT_PATH}` payload: {err}"))
    })
}

fn winner_assignment(winner: &PlanningDuelWinnerArtifact) -> PlanningRoleAssignment {
    PlanningRoleAssignment {
        agent: winner.winner_agent_cli.clone(),
        model: winner.winner_model.clone(),
    }
}

fn winner_slot_for_assignment(
    roles: &PlanningRoles,
    winner: &PlanningRoleAssignment,
) -> Result<PlannerSlot, OrbitError> {
    if roles.planner_a == *winner {
        return Ok(PlannerSlot::PlannerA);
    }
    if roles.planner_b == *winner {
        return Ok(PlannerSlot::PlannerB);
    }
    Err(OrbitError::InvalidInput(format!(
        "winner {}/{} does not match the current planner assignments",
        winner.agent, winner.model
    )))
}

fn normalize_winning_plan_for_task(content: &str) -> String {
    let lines = content.lines().collect::<Vec<_>>();
    if lines.len() <= 1 {
        return content.trim().to_string();
    }
    if parse_planning_duel_signature(content).is_ok() {
        return lines[1..].join("\n").trim().to_string();
    }
    content.trim().to_string()
}

fn find_task_dir(tasks_root: &Path, task_id: &str) -> Result<Option<PathBuf>, OrbitError> {
    if !tasks_root.exists() {
        return Ok(None);
    }

    let mut pending = vec![tasks_root.to_path_buf()];
    let mut matches = Vec::new();

    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).map_err(|err| OrbitError::Io(err.to_string()))? {
            let entry = entry.map_err(|err| OrbitError::Io(err.to_string()))?;
            let path = entry.path();
            if !entry
                .file_type()
                .map_err(|err| OrbitError::Io(err.to_string()))?
                .is_dir()
            {
                continue;
            }

            if path.file_name().and_then(|name| name.to_str()) == Some(task_id) {
                matches.push(path);
                continue;
            }

            pending.push(path);
        }
    }

    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.pop()),
        _ => Err(OrbitError::Execution(format!(
            "found multiple task directories for '{task_id}' while cleaning planning duel artifacts"
        ))),
    }
}

fn cleanup_stale_planning_duel_artifacts<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    task_id: &str,
) -> Result<(), OrbitError> {
    let stale_artifacts = host
        .get_task_artifacts(task_id)?
        .into_iter()
        .filter(|artifact| artifact.path.starts_with(PLANNING_DUEL_ARTIFACT_PREFIX))
        .collect::<Vec<_>>();
    if stale_artifacts.is_empty() {
        return Ok(());
    }

    let tasks_root = host.data_root().join(TASKS_DIR_NAME);
    let task_dir = find_task_dir(&tasks_root, task_id)?.ok_or_else(|| {
        OrbitError::Execution(format!(
            "could not locate task directory for '{task_id}' while cleaning stale planning duel artifacts"
        ))
    })?;
    let artifacts_root = task_dir.join(TASK_ARTIFACTS_DIR_NAME);
    let tool_context = ToolContext {
        cwd: Some(host.data_root().display().to_string()),
        allowed_tools: vec!["fs.delete".to_string()],
        workspace_root: Some(host.data_root().to_path_buf()),
        orbit_root: Some(host.data_root().to_path_buf()),
        task_id: Some(task_id.to_string()),
        ..ToolContext::default()
    };

    for artifact in stale_artifacts {
        let artifact_path = artifacts_root.join(&artifact.path);
        if !artifact_path.exists() {
            return Err(OrbitError::Execution(format!(
                "stale planning duel artifact '{}' is missing on disk for task '{}'",
                artifact.path, task_id
            )));
        }
        let _ = host.run_tool_with_context_and_role(
            "fs.delete",
            json!({ "path": artifact_path.display().to_string() }),
            Role::Admin,
            tool_context.clone(),
        )?;
    }

    Ok(())
}

#[cfg_attr(not(test), allow(dead_code))]
fn summarize_role_metrics(records: &[InvocationRecord]) -> PlanningDuelEfficiency {
    let mut efficiency = PlanningDuelEfficiency {
        invocation_count: records.len() as u64,
        ..PlanningDuelEfficiency::default()
    };

    for record in records {
        efficiency.wall_clock_ms = efficiency.wall_clock_ms.saturating_add(record.duration_ms);
        efficiency.tool_call_count = efficiency
            .tool_call_count
            .saturating_add(record.tool_call_count);
        efficiency.input_tokens = efficiency.input_tokens.saturating_add(record.input_tokens);
        efficiency.cache_read_tokens = efficiency
            .cache_read_tokens
            .saturating_add(record.cache_read_tokens);
        efficiency.cache_create_tokens = efficiency
            .cache_create_tokens
            .saturating_add(record.cache_create_tokens);
        efficiency.output_tokens = efficiency
            .output_tokens
            .saturating_add(record.output_tokens);
        efficiency.total_tokens = efficiency.total_tokens.saturating_add(record.total_tokens);
        efficiency.byte_proxy_total = efficiency.byte_proxy_total.saturating_add(
            record
                .tool_calls
                .iter()
                .map(|tool_call| tool_call.result_bytes)
                .sum::<u64>(),
        );
    }

    efficiency
}

#[cfg_attr(not(test), allow(dead_code))]
fn role_metrics_for_activity<H: RuntimeHost + ?Sized>(
    host: &H,
    job_run_id: &str,
    role_id: &PlanningRoleAssignment,
    activity_id: &str,
) -> Result<PlanningDuelRoleMetrics, OrbitError> {
    let all_records = host.invocation_records_for_job_run_and_activity(job_run_id, activity_id)?;
    let matching_records = all_records
        .iter()
        .filter(|record| {
            record.agent == role_id.agent && record.model.as_deref() == Some(role_id.model.as_str())
        })
        .cloned()
        .collect::<Vec<_>>();

    if matching_records.is_empty() && !all_records.is_empty() {
        return Err(OrbitError::Execution(format!(
            "activity '{activity_id}' for job run '{job_run_id}' did not produce invocations \
             attributed to expected {}/{}",
            role_id.agent, role_id.model
        )));
    }

    Ok(PlanningDuelRoleMetrics {
        agent: role_id.agent.clone(),
        model: role_id.model.clone(),
        activity_id: activity_id.to_string(),
        efficiency: summarize_role_metrics(&matching_records),
    })
}

pub(super) fn select_planning_duel_roles<H: RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let perm = next_permutation();
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

pub(super) fn run_planning_duel<H: RuntimeHost + TaskHost + Sync + ?Sized>(
    host: &H,
    input: &Value,
    debug: bool,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let job_run_id = input_string_field(input, "job_run_id")
        .or_else(|| input_string_field(input, "run_id"))
        .ok_or_else(|| OrbitError::InvalidInput("missing required input.run_id".to_string()))?;

    cleanup_stale_planning_duel_artifacts(host, task_id)?;

    let roles_output = select_planning_duel_roles(host, &json!({ "task_id": task_id }))?;
    let planning_roles = parse_planning_duel_roles(&roles_output)?;

    let planner_activity = planner_activity();
    let planner_a_input = planner_input(task_id);
    let planner_b_input = planner_input(task_id);
    let (planner_a_result, planner_b_result) = std::thread::scope(|scope| {
        let planner_a = planning_roles.planner_a.clone();
        let planner_b = planning_roles.planner_b.clone();
        let planner_activity_a = planner_activity.clone();
        let planner_activity_b = planner_activity.clone();
        let handle_a = scope.spawn(move || {
            host.invoke_activity(
                planner_activity_a,
                &planner_a.agent,
                Some(planner_a.model.as_str()),
                planner_a_input,
                PLANNER_TIMEOUT_SECONDS,
                debug,
            )
        });
        let handle_b = scope.spawn(move || {
            host.invoke_activity(
                planner_activity_b,
                &planner_b.agent,
                Some(planner_b.model.as_str()),
                planner_b_input,
                PLANNER_TIMEOUT_SECONDS,
                debug,
            )
        });
        (
            join_activity_result(handle_a.join(), "planner_a"),
            join_activity_result(handle_b.join(), "planner_b"),
        )
    });
    let planner_a_result = planner_a_result?;
    let planner_b_result = planner_b_result?;

    let planner_artifacts = host.get_task_artifacts(task_id)?;
    let plan_artifacts = planning_duel_plan_artifacts(&planner_artifacts)?;
    let _ = plan_artifact_for_assignment(&plan_artifacts, &planning_roles.planner_a)?;
    let _ = plan_artifact_for_assignment(&plan_artifacts, &planning_roles.planner_b)?;

    let arbiter_result = host.invoke_activity(
        arbiter_activity(),
        &planning_roles.arbiter.agent,
        Some(planning_roles.arbiter.model.as_str()),
        arbiter_input(task_id),
        ARBITER_TIMEOUT_SECONDS,
        debug,
    )?;

    let artifacts_after_arbiter = host.get_task_artifacts(task_id)?;
    let winner = winner_artifact_from_artifacts(&artifacts_after_arbiter)?;

    let role_metrics = BTreeMap::from([
        (
            "planner_a".to_string(),
            role_metrics_from_invocation(
                &planning_roles.planner_a,
                PLANNER_ACTIVITY_ID,
                &planner_a_result,
            ),
        ),
        (
            "planner_b".to_string(),
            role_metrics_from_invocation(
                &planning_roles.planner_b,
                PLANNER_ACTIVITY_ID,
                &planner_b_result,
            ),
        ),
        (
            "arbiter".to_string(),
            role_metrics_from_invocation(
                &planning_roles.arbiter,
                ARBITER_ACTIVITY_ID,
                &arbiter_result,
            ),
        ),
    ]);

    let _ = writeback_planning_duel_task(
        host,
        &json!({
            "task_id": task_id,
            "planning_duel_roles": roles_output["planning_duel_roles"].clone(),
        }),
    )?;
    let _ = record_planning_duel_scores(
        host,
        &json!({
            "task_id": task_id,
            "job_run_id": job_run_id,
            "roles": role_metrics,
        }),
    )?;

    Ok(json!({
        "task_id": task_id,
        "run_id": job_run_id,
        "winner_agent_cli": winner.winner_agent_cli,
        "winner_model": winner.winner_model,
        "recorded": true,
    }))
}

pub(super) fn writeback_planning_duel_task<H: TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let artifacts = host.get_task_artifacts(task_id)?;
    let winner = winner_artifact_from_artifacts(&artifacts)?;
    let winner_assignment = winner_assignment(&winner);
    let plan_artifacts = planning_duel_plan_artifacts(&artifacts)?;
    let winning_artifact = plan_artifact_by_path(&plan_artifacts, &winner.artifact_path)?;
    if winning_artifact.author != winner_assignment {
        return Err(OrbitError::InvalidInput(format!(
            "winner artifact `{}` is authored by {}/{} instead of declared winner {}/{}",
            winner.artifact_path,
            winning_artifact.author.agent,
            winning_artifact.author.model,
            winner_assignment.agent,
            winner_assignment.model
        )));
    }
    let winner_slot = if input.get("planning_duel_roles").is_some() {
        let roles = parse_planning_duel_roles(input)?;
        if winner.arbiter_agent_cli != roles.arbiter.agent
            || winner.arbiter_model != roles.arbiter.model
        {
            return Err(OrbitError::InvalidInput(format!(
                "winner artifact arbiter {}/{} does not match recorded arbiter {}/{}",
                winner.arbiter_agent_cli,
                winner.arbiter_model,
                roles.arbiter.agent,
                roles.arbiter.model
            )));
        }
        Some(winner_slot_for_assignment(&roles, &winner_assignment)?)
    } else {
        None
    };
    let winning_plan = normalize_winning_plan_for_task(&winning_artifact.content);
    let winner_label = winner_slot
        .map(|slot| match slot {
            PlannerSlot::PlannerA => "planner_a",
            PlannerSlot::PlannerB => "planner_b",
        })
        .unwrap_or("planner");

    let status_note = format!(
        "planning duel winner={winner_label} ({}/{})",
        winner_assignment.agent, winner_assignment.model
    );
    let comment_message = format!(
        "Planning duel resolved.\n\nWinner: {winner_label} ({}/{})\n\nRationale: {}\n\nWinning plan persisted to task.plan. Task status was left unchanged.",
        winner_assignment.agent, winner_assignment.model, winner.arbiter_rationale
    );

    host.apply_task_automation_update(
        task_id,
        TaskAutomationUpdate {
            plan: Some(winning_plan),
            status_event: Some("planning_duel_resolved".to_string()),
            status_note: Some(format!(
                "{status_note}; rationale={}",
                winner.arbiter_rationale
            )),
            append_comments: vec![TaskComment {
                at: Utc::now(),
                by: winner.arbiter_agent_cli.clone(),
                message: comment_message,
            }],
            agent: Some(winner_assignment.agent.clone()),
            model: Some(winner_assignment.model.clone()),
            ..TaskAutomationUpdate::default()
        },
    )?;

    Ok(json!({
        "task_id": task_id,
        "status_unchanged": true,
        "winner_agent_cli": winner_assignment.agent,
        "winner_model": winner_assignment.model,
    }))
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn record_planning_duel_efficiency<H: RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let job_run_id = input_string_field(input, "job_run_id")
        .or_else(|| input_string_field(input, "run_id"))
        .ok_or_else(|| OrbitError::InvalidInput("missing required input.run_id".to_string()))?;

    let planning_roles = parse_planning_duel_roles(input)?;

    let planner_a_activity_id = role_activity_id(input, "planner_a")?;
    let planner_b_activity_id = role_activity_id(input, "planner_b")?;
    let arbiter_activity_id = role_activity_id(input, "arbiter")?;

    let planner_a_metrics = role_metrics_for_activity(
        host,
        &job_run_id,
        &planning_roles.planner_a,
        &planner_a_activity_id,
    )?;
    let planner_b_metrics = role_metrics_for_activity(
        host,
        &job_run_id,
        &planning_roles.planner_b,
        &planner_b_activity_id,
    )?;
    let arbiter_metrics = role_metrics_for_activity(
        host,
        &job_run_id,
        &planning_roles.arbiter,
        &arbiter_activity_id,
    )?;

    let roles = BTreeMap::from([
        ("planner_a".to_string(), planner_a_metrics),
        ("planner_b".to_string(), planner_b_metrics),
        ("arbiter".to_string(), arbiter_metrics),
    ]);

    Ok(json!({
        "task_id": task_id,
        "job_run_id": job_run_id,
        "roles": roles,
    }))
}

fn into_efficiency_metrics(value: PlanningDuelEfficiency) -> EfficiencyMetrics {
    let token_usage = TokenUsage {
        input: value.input_tokens,
        cache_read: value.cache_read_tokens,
        cache_create: value.cache_create_tokens,
        output: value.output_tokens,
    };
    let has_exact_tokens = token_usage.input > 0
        || token_usage.cache_read > 0
        || token_usage.cache_create > 0
        || token_usage.output > 0;

    EfficiencyMetrics {
        wall_clock_ms: value.wall_clock_ms,
        tool_call_count: value.tool_call_count.min(u32::MAX as u64) as u32,
        token_usage: has_exact_tokens.then_some(token_usage),
        byte_proxy_total: (!has_exact_tokens && value.byte_proxy_total > 0)
            .then_some(value.byte_proxy_total),
    }
}

pub(super) fn record_planning_duel_scores<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let job_run_id = input_string_field(input, "job_run_id")
        .or_else(|| input_string_field(input, "run_id"))
        .ok_or_else(|| OrbitError::InvalidInput("missing required input.job_run_id".to_string()))?;

    let planner_a_role = serde_json::from_value::<PlanningDuelRoleMetrics>(
        input
            .get("roles")
            .and_then(|roles| roles.get("planner_a"))
            .cloned()
            .ok_or_else(|| {
                OrbitError::InvalidInput("missing required input.roles.planner_a".to_string())
            })?,
    )
    .map_err(|err| OrbitError::InvalidInput(format!("invalid roles.planner_a payload: {err}")))?;
    let planner_b_role = serde_json::from_value::<PlanningDuelRoleMetrics>(
        input
            .get("roles")
            .and_then(|roles| roles.get("planner_b"))
            .cloned()
            .ok_or_else(|| {
                OrbitError::InvalidInput("missing required input.roles.planner_b".to_string())
            })?,
    )
    .map_err(|err| OrbitError::InvalidInput(format!("invalid roles.planner_b payload: {err}")))?;
    let arbiter_role = serde_json::from_value::<PlanningDuelRoleMetrics>(
        input
            .get("roles")
            .and_then(|roles| roles.get("arbiter"))
            .cloned()
            .ok_or_else(|| {
                OrbitError::InvalidInput("missing required input.roles.arbiter".to_string())
            })?,
    )
    .map_err(|err| OrbitError::InvalidInput(format!("invalid roles.arbiter payload: {err}")))?;

    let PlanningDuelRoleMetrics {
        agent: planner_a_agent,
        model: planner_a_model,
        activity_id: _,
        efficiency: planner_a_efficiency,
    } = planner_a_role;
    let PlanningDuelRoleMetrics {
        agent: planner_b_agent,
        model: planner_b_model,
        activity_id: _,
        efficiency: planner_b_efficiency,
    } = planner_b_role;
    let PlanningDuelRoleMetrics {
        agent: arbiter_agent,
        model: arbiter_model,
        activity_id: _,
        efficiency: arbiter_efficiency,
    } = arbiter_role;
    let roles = PlanningRoles {
        planner_a: PlanningRoleAssignment {
            agent: planner_a_agent,
            model: planner_a_model,
        },
        planner_b: PlanningRoleAssignment {
            agent: planner_b_agent,
            model: planner_b_model,
        },
        arbiter: PlanningRoleAssignment {
            agent: arbiter_agent,
            model: arbiter_model,
        },
    };
    let artifacts = host.get_task_artifacts(task_id)?;
    let plan_artifacts = planning_duel_plan_artifacts(&artifacts)?;
    let winner = winner_artifact_from_artifacts(&artifacts)?;
    let winner_assignment = winner_assignment(&winner);
    let winner_plan = plan_artifact_by_path(&plan_artifacts, &winner.artifact_path)?;
    if winner_plan.author != winner_assignment {
        return Err(OrbitError::InvalidInput(format!(
            "winner artifact `{}` is authored by {}/{} instead of declared winner {}/{}",
            winner.artifact_path,
            winner_plan.author.agent,
            winner_plan.author.model,
            winner_assignment.agent,
            winner_assignment.model
        )));
    }
    let winner_slot = winner_slot_for_assignment(&roles, &winner_assignment)?;
    if winner.arbiter_agent_cli != roles.arbiter.agent
        || winner.arbiter_model != roles.arbiter.model
    {
        return Err(OrbitError::InvalidInput(format!(
            "winner artifact arbiter {}/{} does not match recorded arbiter {}/{}",
            winner.arbiter_agent_cli,
            winner.arbiter_model,
            roles.arbiter.agent,
            roles.arbiter.model
        )));
    }
    let planner_a_artifact_path = plan_artifact_for_assignment(&plan_artifacts, &roles.planner_a)?
        .path
        .clone();
    let planner_b_artifact_path = plan_artifact_for_assignment(&plan_artifacts, &roles.planner_b)?
        .path
        .clone();

    let completed_at = Utc::now();
    let run = PlanningDuelRun {
        run_id: job_run_id,
        task_id: task_id.to_string(),
        completed_at,
        roles,
        planner_a_artifact_path,
        planner_b_artifact_path,
        outcome: PlanningOutcome {
            winner: winner_slot,
            arbiter_rationale: winner.arbiter_rationale,
        },
        efficiency: PlanningEfficiency {
            planner_a: into_efficiency_metrics(planner_a_efficiency),
            planner_b: into_efficiency_metrics(planner_b_efficiency),
            arbiter: into_efficiency_metrics(arbiter_efficiency),
        },
    };

    planning_duel_scoreboard::append_run(host.scoreboard_dir(), &run)?;

    Ok(json!({
        "run_id": run.run_id,
        "recorded": true,
    }))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    use chrono::{DateTime, Utc};
    use orbit_store::{
        InvocationQuery, InvocationRecord, InvocationToolCallRecord, planning_duel_scoreboard,
    };
    use orbit_types::{
        AgentModelPair, OrbitError, PlanningRoleAssignment, TokenUsage, ToolCallTrace,
        resolve_agent_model_pair,
    };
    use serde_json::Value;
    use serde_json::json;
    use tempfile::tempdir;

    use crate::context::{ActivityInvocationResult, RuntimeHost, TaskAutomationUpdate, TaskHost};

    use super::{
        PlanningDuelEfficiency, clear_test_permutations, push_test_permutations,
        record_planning_duel_efficiency, record_planning_duel_scores, run_planning_duel,
        select_planning_duel_roles, summarize_role_metrics, writeback_planning_duel_task,
    };

    #[derive(Default)]
    struct StubTaskHost {
        updates: Mutex<Vec<(String, TaskAutomationUpdate)>>,
        artifacts: Mutex<HashMap<String, Vec<orbit_types::TaskArtifact>>>,
    }

    impl TaskHost for StubTaskHost {
        fn get_task(&self, _task_id: &str) -> Result<orbit_types::Task, OrbitError> {
            unimplemented!("not used")
        }

        fn get_task_artifacts(
            &self,
            task_id: &str,
        ) -> Result<Vec<orbit_types::TaskArtifact>, OrbitError> {
            Ok(self
                .artifacts
                .lock()
                .unwrap()
                .get(task_id)
                .cloned()
                .unwrap_or_default())
        }

        fn list_tasks_filtered(
            &self,
            _status: Option<orbit_types::TaskStatus>,
            _priority: Option<orbit_types::TaskPriority>,
            _parent_id: Option<&str>,
            _batch_id: Option<&str>,
        ) -> Result<Vec<orbit_types::Task>, OrbitError> {
            Ok(Vec::new())
        }

        fn start_task(
            &self,
            _task_id: &str,
            _note: Option<String>,
            _comment: Option<String>,
        ) -> Result<orbit_types::Task, OrbitError> {
            unimplemented!()
        }

        fn update_task_from_activity(
            &self,
            _task_id: &str,
            _status: orbit_types::TaskStatus,
            _execution_summary: Option<String>,
            _comment: Option<String>,
            _note: Option<String>,
        ) -> Result<orbit_types::Task, OrbitError> {
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

    struct StubRuntimeHost {
        data_root: tempfile::TempDir,
        scoreboard_dir: tempfile::TempDir,
        pair_overrides: HashMap<String, AgentModelPair>,
        queries: Mutex<Vec<(String, String)>>,
        records: Mutex<HashMap<(String, String), Vec<InvocationRecord>>>,
        artifacts: Mutex<HashMap<String, Vec<orbit_types::TaskArtifact>>>,
        updates: Mutex<Vec<(String, TaskAutomationUpdate)>>,
        invocations: Mutex<Vec<(String, String)>>,
        operations: Mutex<Vec<String>>,
        active_planners: AtomicUsize,
        max_active_planners: AtomicUsize,
    }

    impl StubRuntimeHost {
        fn new() -> Self {
            Self {
                data_root: tempdir().expect("tempdir"),
                scoreboard_dir: tempdir().expect("tempdir"),
                pair_overrides: HashMap::new(),
                queries: Mutex::new(Vec::new()),
                records: Mutex::new(HashMap::new()),
                artifacts: Mutex::new(HashMap::new()),
                updates: Mutex::new(Vec::new()),
                invocations: Mutex::new(Vec::new()),
                operations: Mutex::new(Vec::new()),
                active_planners: AtomicUsize::new(0),
                max_active_planners: AtomicUsize::new(0),
            }
        }

        fn with_pair_override(mut self, family: &str, orchestrator: &str, helper: &str) -> Self {
            self.pair_overrides.insert(
                family.to_string(),
                AgentModelPair::new(orchestrator, helper),
            );
            self
        }

        fn write_task_artifacts(
            &self,
            task_dir_relative: &str,
            artifacts: &[orbit_types::TaskArtifact],
        ) {
            let artifacts_root = self
                .data_root
                .path()
                .join(task_dir_relative)
                .join(super::TASK_ARTIFACTS_DIR_NAME);
            fs::create_dir_all(&artifacts_root).expect("create artifacts root");
            for artifact in artifacts {
                let destination = artifacts_root.join(&artifact.path);
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent).expect("create artifact parent");
                }
                fs::write(&destination, &artifact.content).expect("write artifact");
            }
        }
    }

    impl TaskHost for StubRuntimeHost {
        fn get_task(&self, _task_id: &str) -> Result<orbit_types::Task, OrbitError> {
            unimplemented!()
        }

        fn get_task_artifacts(
            &self,
            task_id: &str,
        ) -> Result<Vec<orbit_types::TaskArtifact>, OrbitError> {
            Ok(self
                .artifacts
                .lock()
                .unwrap()
                .get(task_id)
                .cloned()
                .unwrap_or_default())
        }

        fn list_tasks_filtered(
            &self,
            _status: Option<orbit_types::TaskStatus>,
            _priority: Option<orbit_types::TaskPriority>,
            _parent_id: Option<&str>,
            _batch_id: Option<&str>,
        ) -> Result<Vec<orbit_types::Task>, OrbitError> {
            Ok(Vec::new())
        }

        fn start_task(
            &self,
            _task_id: &str,
            _note: Option<String>,
            _comment: Option<String>,
        ) -> Result<orbit_types::Task, OrbitError> {
            unimplemented!()
        }

        fn update_task_from_activity(
            &self,
            _task_id: &str,
            _status: orbit_types::TaskStatus,
            _execution_summary: Option<String>,
            _comment: Option<String>,
            _note: Option<String>,
        ) -> Result<orbit_types::Task, OrbitError> {
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

    impl RuntimeHost for StubRuntimeHost {
        fn record_event(&self, _event: orbit_types::OrbitEvent) -> Result<(), OrbitError> {
            unimplemented!()
        }

        fn repo_root(&self) -> Result<String, OrbitError> {
            unimplemented!()
        }

        fn data_root(&self) -> &std::path::Path {
            self.data_root.path()
        }

        fn run_job_now_with_input_debug(
            &self,
            _job_id: &str,
            _input: Value,
            _debug: bool,
        ) -> Result<crate::JobRunResult, OrbitError> {
            unimplemented!()
        }

        fn validate_activity_target_exists(
            &self,
            _target_type: orbit_types::JobTargetType,
            _target_id: &str,
        ) -> Result<orbit_types::Activity, OrbitError> {
            unimplemented!()
        }

        fn get_job(&self, _job_id: &str) -> Result<Option<orbit_types::Job>, OrbitError> {
            unimplemented!()
        }

        fn invocation_records(
            &self,
            query: InvocationQuery,
        ) -> Result<Vec<InvocationRecord>, OrbitError> {
            let job_run_id = query.job_run_id.unwrap_or_default();
            let activity_id = query.activity_id.unwrap_or_default();
            self.queries
                .lock()
                .unwrap()
                .push((job_run_id.clone(), activity_id.clone()));
            Ok(self
                .records
                .lock()
                .unwrap()
                .get(&(job_run_id, activity_id))
                .cloned()
                .unwrap_or_default())
        }

        fn run_tool_with_context_and_role(
            &self,
            name: &str,
            input: Value,
            role: orbit_types::Role,
            tool_context: orbit_tools::ToolContext,
        ) -> Result<Value, OrbitError> {
            self.operations
                .lock()
                .unwrap()
                .push(format!("tool:{name}"));

            match name {
                "fs.delete" => {
                    assert_eq!(role, orbit_types::Role::Admin);
                    assert_eq!(tool_context.allowed_tools, vec!["fs.delete".to_string()]);
                    assert_eq!(
                        tool_context.workspace_root.as_deref(),
                        Some(self.data_root.path())
                    );

                    let path = input
                        .get("path")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            OrbitError::InvalidInput("missing required input.path".to_string())
                        })?;
                    let path = std::path::PathBuf::from(path);
                    fs::remove_file(&path).map_err(|err| OrbitError::Io(err.to_string()))?;

                    let suffix = std::path::Path::new(super::TASK_ARTIFACTS_DIR_NAME);
                    let mut artifacts = self.artifacts.lock().unwrap();
                    for task_artifacts in artifacts.values_mut() {
                        task_artifacts.retain(|artifact| {
                            !path.ends_with(suffix.join(&artifact.path))
                        });
                    }

                    Ok(json!({
                        "deleted": true,
                        "path": path.display().to_string(),
                    }))
                }
                other => Err(OrbitError::Execution(format!(
                    "unexpected tool invocation '{other}'"
                ))),
            }
        }

        fn invoke_activity(
            &self,
            activity: orbit_types::Activity,
            agent_cli: &str,
            model: Option<&str>,
            input: Value,
            _timeout_seconds: u64,
            _debug: bool,
        ) -> Result<ActivityInvocationResult, OrbitError> {
            let task_id = input
                .get("task_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    OrbitError::InvalidInput("missing required input.task_id".to_string())
                })?
                .to_string();
            self.invocations
                .lock()
                .unwrap()
                .push((activity.id.clone(), agent_cli.to_string()));
            self.operations
                .lock()
                .unwrap()
                .push(format!("activity:{}:{agent_cli}", activity.id));

            match activity.id.as_str() {
                super::PLANNER_ACTIVITY_ID => {
                    let model = model.ok_or_else(|| {
                        OrbitError::InvalidInput("planner invocation requires model".to_string())
                    })?;
                    let active = self.active_planners.fetch_add(1, Ordering::SeqCst) + 1;
                    self.max_active_planners.fetch_max(active, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(50));
                    self.active_planners.fetch_sub(1, Ordering::SeqCst);

                    self.artifacts.lock().unwrap().entry(task_id).or_default().push(
                        orbit_types::TaskArtifact {
                            path: format!("planning-duel/{agent_cli}-{model}.md"),
                            content: format!(
                                "*authored by: {agent_cli} / {model}*\n## Plan\n- planner from {agent_cli}"
                            ),
                        },
                    );

                    Ok(ActivityInvocationResult {
                        response_json: None,
                        // Keep the trace duration shared so tests catch any
                        // accidental fallback to `invocation_trace.duration_ms`.
                        invocation_trace: sample_trace(
                            90,
                            if agent_cli == "codex" { 11 } else { 17 },
                            if agent_cli == "codex" { 5 } else { 8 },
                            if agent_cli == "codex" { 48 } else { 96 },
                        ),
                        exit_code: Some(0),
                        duration_ms: if agent_cli == "codex" { 40 } else { 65 },
                    })
                }
                super::ARBITER_ACTIVITY_ID => {
                    let _model = model.ok_or_else(|| {
                        OrbitError::InvalidInput("arbiter invocation requires model".to_string())
                    })?;
                    let mut artifacts = self.artifacts.lock().unwrap();
                    let task_artifacts = artifacts.entry(task_id).or_default();
                    let planner_count = task_artifacts
                        .iter()
                        .filter(|artifact| artifact.path.ends_with(".md"))
                        .count();
                    if planner_count != 2 {
                        return Err(OrbitError::Execution(format!(
                            "arbiter expected 2 planner artifacts, found {planner_count}"
                        )));
                    }
                    task_artifacts.push(winner_artifact());

                    Ok(ActivityInvocationResult {
                        response_json: None,
                        invocation_trace: sample_trace(25, 9, 4, 32),
                        exit_code: Some(0),
                        duration_ms: 25,
                    })
                }
                other => Err(OrbitError::Execution(format!(
                    "unexpected activity invocation '{other}'"
                ))),
            }
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
            unimplemented!()
        }

        fn resolved_agent_model_pair(&self, agent_cli: &str) -> Option<AgentModelPair> {
            self.pair_overrides
                .get(agent_cli)
                .cloned()
                .or_else(|| resolve_agent_model_pair(agent_cli))
        }

        fn scoring_enabled(&self) -> bool {
            false
        }

        fn graph_editing(&self) -> bool {
            false
        }

        fn scoreboard_dir(&self) -> &std::path::Path {
            self.scoreboard_dir.path()
        }
    }

    fn sample_trace(
        duration_ms: u64,
        input_tokens: u64,
        output_tokens: u64,
        result_bytes: u64,
    ) -> orbit_types::InvocationTrace {
        orbit_types::InvocationTrace {
            usage: TokenUsage {
                input: input_tokens,
                output: output_tokens,
                ..TokenUsage::default()
            },
            tool_calls: vec![ToolCallTrace {
                seq: 1,
                tool_name: "orbit.graph.show".to_string(),
                result_bytes,
                result_payload: None,
            }],
            duration_ms,
        }
    }

    fn record(
        id: i64,
        ts: &str,
        job_run_id: &str,
        activity_id: &str,
        agent: &str,
        model: &str,
        duration_ms: u64,
        input_tokens: u64,
        output_tokens: u64,
        tool_calls: &[(&str, u64)],
    ) -> InvocationRecord {
        InvocationRecord {
            id,
            ts: DateTime::parse_from_rfc3339(ts)
                .unwrap()
                .with_timezone(&Utc),
            job_run_id: job_run_id.to_string(),
            activity_id: activity_id.to_string(),
            agent: agent.to_string(),
            model: Some(model.to_string()),
            duration_ms,
            input_tokens,
            cache_read_tokens: 0,
            cache_create_tokens: 0,
            output_tokens,
            total_tokens: input_tokens.saturating_add(output_tokens),
            tool_call_count: tool_calls.len() as u64,
            task_ids: vec!["T1".to_string()],
            tool_calls: tool_calls
                .iter()
                .enumerate()
                .map(
                    |(seq, (tool_name, result_bytes))| InvocationToolCallRecord {
                        invocation_id: id,
                        seq: seq as u64 + 1,
                        tool_name: tool_name.to_string(),
                        result_bytes: *result_bytes,
                    },
                )
                .collect(),
        }
    }

    fn planning_roles_json() -> Value {
        json!({
            "planner_a": {
                "agent": "codex",
                "model": "gpt-5.4"
            },
            "planner_b": {
                "agent": "claude",
                "model": "opus-4.6"
            },
            "arbiter": {
                "agent": "gemini",
                "model": "gemini-3.1-pro-preview"
            }
        })
    }

    fn planner_artifacts() -> Vec<orbit_types::TaskArtifact> {
        vec![
            orbit_types::TaskArtifact {
                path: "planning-duel/codex-gpt-5.4.md".to_string(),
                content: "*authored by: codex / gpt-5.4*\n## Plan\n- planner a".to_string(),
            },
            orbit_types::TaskArtifact {
                path: "planning-duel/claude-opus-4.6.md".to_string(),
                content: "*authored by: claude / opus-4.6*\n## Plan\n- planner b".to_string(),
            },
        ]
    }

    fn winner_artifact() -> orbit_types::TaskArtifact {
        orbit_types::TaskArtifact {
            path: super::WINNER_ARTIFACT_PATH.to_string(),
            content: serde_json::to_string(&json!({
                "winner_agent_cli": "claude",
                "winner_model": "opus-4.6",
                "artifact_path": "planning-duel/claude-opus-4.6.md",
                "arbiter_agent_cli": "gemini",
                "arbiter_model": "gemini-3.1-pro-preview",
                "arbiter_rationale": "planner_b grounded the writeback path more clearly"
            }))
            .expect("winner json"),
        }
    }

    #[test]
    fn select_planning_duel_roles_returns_distinct_assignments() {
        clear_test_permutations();
        push_test_permutations([[0, 1, 2]]);

        let host = StubRuntimeHost::new();
        let out = select_planning_duel_roles(&host, &json!({ "task_id": "T1" })).unwrap();
        assert_eq!(out["planner_a_agent_cli"], json!("codex"));
        assert_eq!(out["planner_b_agent_cli"], json!("claude"));
        assert_eq!(out["arbiter_agent_cli"], json!("gemini"));
        assert_eq!(
            out["planning_duel_roles"]["planner_a"]["agent"],
            json!("codex")
        );
        assert_eq!(
            out["planning_duel_roles"]["arbiter"]["agent"],
            json!("gemini")
        );
    }

    #[test]
    fn select_planning_duel_roles_prefers_host_resolved_model_pairs() {
        clear_test_permutations();
        push_test_permutations([[1, 0, 2]]);

        let host = StubRuntimeHost::new().with_pair_override("claude", "opus-4.7", "sonnet-4.7");
        let out = select_planning_duel_roles(&host, &json!({ "task_id": "T1" })).unwrap();

        assert_eq!(out["planner_a_agent_cli"], json!("claude"));
        assert_eq!(out["planner_a_model"], json!("opus-4.7"));
        assert_eq!(
            out["planning_duel_roles"]["planner_a"]["model"],
            json!("opus-4.7")
        );
    }

    #[test]
    fn writeback_planning_duel_task_preserves_status_and_records_history() {
        let host = StubTaskHost::default();
        host.artifacts.lock().unwrap().insert("T1".to_string(), {
            let mut artifacts = planner_artifacts();
            artifacts.push(winner_artifact());
            artifacts
        });
        let out = writeback_planning_duel_task(
            &host,
            &json!({
                "task_id": "T1",
                "planning_duel_roles": planning_roles_json(),
            }),
        )
        .unwrap();

        assert_eq!(out["status_unchanged"], json!(true));
        let updates = host.updates.lock().unwrap();
        assert_eq!(updates.len(), 1);
        let update = &updates[0].1;
        assert_eq!(update.plan.as_deref(), Some("## Plan\n- planner b"));
        assert_eq!(update.status, None);
        assert_eq!(
            update.status_event.as_deref(),
            Some("planning_duel_resolved")
        );
        assert!(
            matches!(
                update.status_note.as_deref(),
                Some(note)
                    if note.contains("rationale=planner_b grounded the writeback path more clearly")
            )
        );
        assert_eq!(update.agent.as_deref(), Some("claude"));
        assert_eq!(update.append_comments.len(), 1);
        assert_eq!(update.append_comments[0].by, "gemini");
        assert!(update.append_comments[0]
            .message
            .contains("Task status was left unchanged."));
    }

    #[test]
    fn run_planning_duel_invokes_planners_in_parallel_then_records_winner() {
        clear_test_permutations();
        push_test_permutations([[0, 1, 2]]);

        let host = StubRuntimeHost::new();
        let out = run_planning_duel(
            &host,
            &json!({
                "task_id": "T1",
                "run_id": "run-42",
            }),
            false,
        )
        .expect("planning duel succeeds");

        assert_eq!(out["run_id"], json!("run-42"));
        assert_eq!(out["winner_agent_cli"], json!("claude"));
        assert_eq!(out["winner_model"], json!("opus-4.6"));

        let invocations = host.invocations.lock().unwrap().clone();
        assert_eq!(invocations.len(), 3);
        assert_eq!(invocations[0].0, super::PLANNER_ACTIVITY_ID);
        assert_eq!(invocations[1].0, super::PLANNER_ACTIVITY_ID);
        assert_eq!(invocations[2].0, super::ARBITER_ACTIVITY_ID);
        assert!(
            host.max_active_planners.load(Ordering::SeqCst) >= 2,
            "expected planner invocations to overlap"
        );

        let updates = host.updates.lock().unwrap();
        assert_eq!(updates.len(), 1);
        let update = &updates[0].1;
        assert_eq!(update.status, None);
        assert_eq!(
            update.plan.as_deref(),
            Some("## Plan\n- planner from claude")
        );

        let runs = planning_duel_scoreboard::load_runs(host.scoreboard_dir.path())
            .expect("scoreboard loads");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].task_id, "T1");
        assert_eq!(runs[0].outcome.winner, orbit_types::PlannerSlot::PlannerB);
        assert_eq!(runs[0].roles.arbiter.agent, "gemini");
        assert_eq!(runs[0].efficiency.planner_a.wall_clock_ms, 40);
        assert_eq!(runs[0].efficiency.planner_b.wall_clock_ms, 65);
        assert_eq!(runs[0].efficiency.arbiter.wall_clock_ms, 25);

        let artifacts = host
            .artifacts
            .lock()
            .unwrap()
            .get("T1")
            .cloned()
            .expect("task artifacts");
        assert_eq!(
            artifacts
                .iter()
                .filter(|artifact| artifact.path.ends_with(".md"))
                .count(),
            2
        );
        assert!(
            artifacts
                .iter()
                .any(|artifact| artifact.path == super::WINNER_ARTIFACT_PATH)
        );
    }

    #[test]
    fn run_planning_duel_cleans_stale_artifacts_before_new_planners() {
        clear_test_permutations();
        push_test_permutations([[0, 1, 2]]);

        let task_id = "T20260412-2240";
        let host = StubRuntimeHost::new();
        let stale_artifacts = vec![
            orbit_types::TaskArtifact {
                path: "planning-duel/gemini-gemini-3.1-pro-preview.md".to_string(),
                content: "*authored by: gemini / gemini-3.1-pro-preview*\n## Plan\n- stale planner".to_string(),
            },
            winner_artifact(),
        ];
        host.artifacts
            .lock()
            .unwrap()
            .insert(task_id.to_string(), stale_artifacts.clone());
        host.write_task_artifacts(
            &format!("tasks/done/2026-04/{task_id}"),
            &stale_artifacts,
        );

        let out = run_planning_duel(
            &host,
            &json!({
                "task_id": task_id,
                "run_id": "run-clean",
            }),
            false,
        )
        .expect("planning duel succeeds after stale cleanup");

        assert_eq!(out["winner_agent_cli"], json!("claude"));
        let operations = host.operations.lock().unwrap().clone();
        assert_eq!(operations.len(), 5);
        assert!(operations[0].starts_with("tool:fs.delete"));
        assert!(operations[1].starts_with("tool:fs.delete"));
        assert!(operations[2].starts_with("activity:propose_duel_plan:"));
        assert!(operations[3].starts_with("activity:propose_duel_plan:"));
        assert_eq!(operations[4], "activity:arbitrate_duel_plan:gemini");

        let artifacts = host.get_task_artifacts(task_id).expect("artifacts after duel");
        assert_eq!(artifacts.len(), 3);
        assert!(
            artifacts
                .iter()
                .all(|artifact| artifact.path != "planning-duel/gemini-gemini-3.1-pro-preview.md")
        );
        assert!(
            !host.data_root
                .path()
                .join(format!(
                    "tasks/done/2026-04/{task_id}/artifacts/planning-duel/gemini-gemini-3.1-pro-preview.md"
                ))
                .exists()
        );
    }

    #[test]
    fn record_planning_duel_efficiency_queries_invocation_traces_by_role() {
        let host = StubRuntimeHost::new();
        host.records.lock().unwrap().insert(
            ("run-1".to_string(), "activity-a".to_string()),
            vec![
                record(
                    1,
                    "2026-04-11T12:00:00Z",
                    "run-1",
                    "activity-a",
                    "codex",
                    "gpt-5.4",
                    25,
                    10,
                    3,
                    &[("orbit.graph.search", 40), ("orbit.graph.refs", 20)],
                ),
                record(
                    2,
                    "2026-04-11T12:01:00Z",
                    "run-1",
                    "activity-a",
                    "codex",
                    "gpt-5.4",
                    35,
                    5,
                    7,
                    &[("orbit.graph.show", 50)],
                ),
            ],
        );
        host.records.lock().unwrap().insert(
            ("run-1".to_string(), "activity-b".to_string()),
            vec![record(
                3,
                "2026-04-11T12:02:00Z",
                "run-1",
                "activity-b",
                "claude",
                "opus-4.6",
                60,
                12,
                8,
                &[("orbit.graph.overview", 60)],
            )],
        );
        host.records.lock().unwrap().insert(
            ("run-1".to_string(), "activity-c".to_string()),
            vec![record(
                4,
                "2026-04-11T12:03:00Z",
                "run-1",
                "activity-c",
                "gemini",
                "gemini-3.1-pro-preview",
                90,
                1,
                2,
                &[("orbit.graph.search", 70)],
            )],
        );

        let out = record_planning_duel_efficiency(
            &host,
            &json!({
                "task_id": "T1",
                "job_run_id": "run-1",
                "planning_duel_roles": planning_roles_json(),
                "planner_a_activity_id": "activity-a",
                "planner_b_activity_id": "activity-b",
                "arbiter_activity_id": "activity-c",
            }),
        )
        .unwrap();

        assert_eq!(out["task_id"], json!("T1"));
        assert_eq!(
            out["roles"]["planner_a"]["efficiency"]["invocation_count"],
            json!(2)
        );
        assert_eq!(
            out["roles"]["planner_a"]["efficiency"]["wall_clock_ms"],
            json!(60)
        );
        assert_eq!(
            out["roles"]["planner_a"]["efficiency"]["tool_call_count"],
            json!(3)
        );
        assert_eq!(
            out["roles"]["planner_a"]["efficiency"]["byte_proxy_total"],
            json!(110)
        );
        assert_eq!(
            out["roles"]["planner_b"]["efficiency"]["byte_proxy_total"],
            json!(60)
        );
        assert_eq!(
            out["roles"]["arbiter"]["efficiency"]["wall_clock_ms"],
            json!(90)
        );
        assert_eq!(host.queries.lock().unwrap().len(), 3);
    }

    #[test]
    fn record_planning_duel_scores_appends_scoreboard_entry() {
        let host = StubRuntimeHost::new();
        host.artifacts.lock().unwrap().insert("T1".to_string(), {
            let mut artifacts = planner_artifacts();
            artifacts.push(winner_artifact());
            artifacts
        });

        let out = record_planning_duel_scores(
            &host,
            &json!({
                "job_run_id": "run-77",
                "task_id": "T1",
                "roles": {
                    "planner_a": {
                        "agent": "codex",
                        "model": "gpt-5.4",
                        "activity_id": "propose_duel_plan",
                        "efficiency": {
                            "invocation_count": 1,
                            "wall_clock_ms": 1200,
                            "tool_call_count": 3,
                            "input_tokens": 20,
                            "cache_read_tokens": 0,
                            "cache_create_tokens": 0,
                            "output_tokens": 10,
                            "total_tokens": 30,
                            "byte_proxy_total": 0
                        }
                    },
                    "planner_b": {
                        "agent": "claude",
                        "model": "opus-4.6",
                        "activity_id": "propose_duel_plan",
                        "efficiency": {
                            "invocation_count": 1,
                            "wall_clock_ms": 1500,
                            "tool_call_count": 4,
                            "input_tokens": 0,
                            "cache_read_tokens": 0,
                            "cache_create_tokens": 0,
                            "output_tokens": 0,
                            "total_tokens": 0,
                            "byte_proxy_total": 4096
                        }
                    },
                    "arbiter": {
                        "agent": "gemini",
                        "model": "gemini-3.1-pro-preview",
                        "activity_id": "arbitrate_duel_plan",
                        "efficiency": {
                            "invocation_count": 1,
                            "wall_clock_ms": 900,
                            "tool_call_count": 2,
                            "input_tokens": 15,
                            "cache_read_tokens": 0,
                            "cache_create_tokens": 0,
                            "output_tokens": 5,
                            "total_tokens": 20,
                            "byte_proxy_total": 0
                        }
                    }
                }
            }),
        )
        .unwrap();

        assert_eq!(out["recorded"], json!(true));
        let runs = planning_duel_scoreboard::load_runs(host.scoreboard_dir()).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "run-77");
        assert_eq!(runs[0].task_id, "T1");
        assert_eq!(runs[0].outcome.winner, orbit_types::PlannerSlot::PlannerB);
        assert_eq!(runs[0].roles.planner_b.agent, "claude");
        assert_eq!(
            runs[0].planner_b_artifact_path,
            "planning-duel/claude-opus-4.6.md"
        );
        assert_eq!(runs[0].efficiency.planner_a.token_total(), Some(30));
        assert_eq!(runs[0].efficiency.planner_b.byte_proxy_total(), Some(4096));
    }

    #[test]
    fn record_planning_duel_efficiency_supports_shared_planner_activity_id() {
        let host = StubRuntimeHost::new();
        host.records.lock().unwrap().insert(
            ("run-1".to_string(), "shared-activity".to_string()),
            vec![
                record(
                    1,
                    "2026-04-11T12:00:00Z",
                    "run-1",
                    "shared-activity",
                    "codex",
                    "gpt-5.4",
                    25,
                    10,
                    3,
                    &[("orbit.graph.search", 40)],
                ),
                record(
                    2,
                    "2026-04-11T12:01:00Z",
                    "run-1",
                    "shared-activity",
                    "claude",
                    "opus-4.6",
                    45,
                    0,
                    0,
                    &[("orbit.graph.show", 80)],
                ),
            ],
        );

        let out = record_planning_duel_efficiency(
            &host,
            &json!({
                "task_id": "T1",
                "job_run_id": "run-1",
                "planning_duel_roles": planning_roles_json(),
                "planner_activity_id": "shared-activity",
                "arbiter_activity_id": "activity-c",
            }),
        )
        .unwrap();

        assert_eq!(
            out["roles"]["planner_a"]["efficiency"]["wall_clock_ms"],
            json!(25)
        );
        assert_eq!(
            out["roles"]["planner_b"]["efficiency"]["byte_proxy_total"],
            json!(80)
        );
    }

    #[test]
    fn record_planning_duel_efficiency_rejects_misattributed_invocations() {
        let host = StubRuntimeHost::new();
        host.records.lock().unwrap().insert(
            ("run-1".to_string(), "activity-a".to_string()),
            vec![record(
                1,
                "2026-04-11T12:00:00Z",
                "run-1",
                "activity-a",
                "claude",
                "opus-4.6",
                25,
                10,
                3,
                &[("orbit.graph.search", 40)],
            )],
        );

        let err = record_planning_duel_efficiency(
            &host,
            &json!({
                "task_id": "T1",
                "job_run_id": "run-1",
                "planning_duel_roles": planning_roles_json(),
                "planner_activity_id": "activity-a",
                "arbiter_activity_id": "activity-c",
            }),
        )
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("did not produce invocations attributed to expected codex/gpt-5.4")
        );
    }

    #[test]
    fn summarize_role_metrics_accumulates_totals() {
        let metrics = summarize_role_metrics(&[
            record(
                1,
                "2026-04-11T12:00:00Z",
                "run-1",
                "activity-a",
                "codex",
                "gpt-5.4",
                10,
                4,
                2,
                &[("tool-a", 5)],
            ),
            record(
                2,
                "2026-04-11T12:00:01Z",
                "run-1",
                "activity-a",
                "codex",
                "gpt-5.4",
                20,
                6,
                8,
                &[("tool-b", 7)],
            ),
        ]);

        assert_eq!(
            metrics,
            PlanningDuelEfficiency {
                invocation_count: 2,
                wall_clock_ms: 30,
                tool_call_count: 2,
                input_tokens: 10,
                cache_read_tokens: 0,
                cache_create_tokens: 0,
                output_tokens: 10,
                total_tokens: 20,
                byte_proxy_total: 12,
            }
        );
    }

    #[test]
    fn role_metrics_from_invocation_uses_explicit_activity_duration() {
        let metrics = super::role_metrics_from_invocation(
            &PlanningRoleAssignment {
                agent: "codex".to_string(),
                model: "gpt-5.4".to_string(),
            },
            super::PLANNER_ACTIVITY_ID,
            &ActivityInvocationResult {
                response_json: None,
                invocation_trace: sample_trace(999, 11, 5, 48),
                exit_code: Some(0),
                duration_ms: 40,
            },
        );

        assert_eq!(metrics.efficiency.wall_clock_ms, 40);
        assert_eq!(metrics.efficiency.tool_call_count, 1);
        assert_eq!(metrics.efficiency.total_tokens, 16);
        assert_eq!(metrics.efficiency.byte_proxy_total, 48);
    }
}
