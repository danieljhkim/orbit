use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Utc;
use orbit_common::types::{
    Activity, AgentFamily, OrbitError, PlanningRoleAssignment, PlanningRoles, RoleSlot,
};
use serde_json::{Value, json};

use crate::context::RuntimeHost;

use crate::executor::automation::input::{input_string_field, required_input_string};

use super::super::{role_permutation_at, validate_role_permutation};

pub(super) const PLANNER_ACTIVITY_ID: &str = "propose_duel_plan";
pub(super) const ARBITER_ACTIVITY_ID: &str = "arbitrate_duel_plan";
pub(super) const PLANNER_TIMEOUT_SECONDS: u64 = 1800;
pub(super) const ARBITER_TIMEOUT_SECONDS: u64 = 900;

const PLANNING_DUEL_INSTRUCTION: &str = r###"Only use skills listed in this activity's skill_refs. Ignore all others.
You are a PLANNER in an Orbit planning duel. Inspect the task and surrounding
code, draft one implementation-ready proposal, and persist it to the task's
`artifacts/` directory. Do not edit source files, open PRs, or rely on your
structured response as the workflow handoff.

Steps:
1. Load the task:
   - Call orbit.task.show with input: {"id": "<task_id>"} to fetch the task title,
     description, plan, acceptance_criteria, context_files, and workspace_path.

2. Determine your artifact path from the active slot:
   - Your active agent family is `{{agent_family}}`.
   - Your active planning-duel slot is in input.planning_duel_slot.
   - Your plan artifact path must be `planning-duel/<slot>.md`.

3. Gather context with the graph surface BEFORE drafting. No single tool is
   sufficient — show returns bodies; refs, callees, and search return the call
   and import graph that bodies alone miss. You must:
   - Start with orbit.graph.overview over the task's directories and
     orbit.graph.show on each task.context_files selector for breadth.
   - For every symbol the task proposes to move, rename, remove, or add: call
     orbit.graph.refs on it to enumerate callers and consumers BY NAME, and
     orbit.graph.impact to bound the blast radius.
   - For every module boundary you are drawing (new crate, new module, extracted
     file, renamed type): call orbit.graph.search to find call sites of moved
     types and helpers across the workspace, and orbit.graph.deps to confirm the
     import direction.
   - Use orbit.graph.show for full symbol bodies you need to read.
   - Use orbit.graph.overview to map an unfamiliar area first.
   - fs.read is a fallback only for selectors the graph cannot resolve (raw
     YAML, Markdown, assets the graph does not index, or unresolved selectors
     the graph reports).
   - If you discover pub(crate) imports, helper coupling, call sites, or
     dependency edges not reflected in the task description, treat them as
     hidden coupling — they belong in step 1 of your plan body.

4. Draft exactly one proposal as markdown:
   - Include these sections:
     ## Plan
     ## Context Files
     ## Risks
   - Ignore any existing planner artifact for the other role. Your proposal must
     be independently reasoned.
   - The plan MUST:
     - Name every symbol being moved, renamed, removed, or added (functions,
       types, modules, constants) BY IDENTIFIER, not by category.
     - Enumerate the consumers and call sites discovered via orbit.graph.refs
       and orbit.graph.search BY NAME.
     - Specify exact verification commands the implementer should run — e.g.
       `cargo build -p <crate>`, `cargo test -p <crate> <test_name>`,
       `make ci-fast`, `rg '<symbol>' <path>`, or
       `curl -s http://localhost:<port>/<route>` — that prove the change works.
     - If hidden coupling exists (imports, helpers, call sites the task
       description did not name), open the plan with step 1 enumerating it.
   - The plan MUST NOT:
     - Use hedge language: "this should just work", "should compile", "verify
       it continues to compile", "we must verify", "this just works", or
       similar. Replace each with the exact command above that proves it.
     - Defer evidence to the implementer ("the implementer will discover X")
       when orbit.graph.* could have surfaced X now.
   - Length is not the goal. Named identifiers, enumerated consumers, and exact
     verification commands are.

5. Persist the proposal as a task artifact:
   - Use orbit.duel.plan.add to write the artifact under the slot-derived path. Orbit stamps the signature line.
   - Exact example:
     {"id":"<task_id>","planning_duel_slot":"planner_a","content":"## Plan\n..."}

6. Stay narrowly scoped:
   - Do not edit source files, update task.plan, or touch PR state.
   - The only permitted mutation is writing your own planner artifact via orbit.duel.plan.add.

7. Structured output is optional:
   - The workflow does not depend on your response payload. Persist the artifact correctly even if you return null."###;
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
   - The first line of each planner artifact must be `*authored by: <family> / <slot>*`.
   - Parse those lines to recover each planner's family and slot.
   - The artifact signature is the canonical planner identity source.

4. Use the graph surface to verify claims:
   - Prefer orbit.graph.overview, orbit.graph.search, orbit.graph.refs, orbit.graph.show,
     and orbit.graph.impact for spot checks against the codebase.
   - Fall back to fs.read only when the graph does not have enough knowledge.

5. Decide the winner:
   - Choose the artifact proposal that is more feasible, complete, scoped, and aligned
     with the current codebase.
   - Keep a short `arbiter_rationale` that explains why the winning proposal is better.

6. Persist the winner marker:
   - Use orbit.duel.plan.winner to write `planning-duel/winner.json`.
   - Exact example:
     {"id":"<task_id>","winner_slot":"planner_a","arbiter_rationale":"More concrete writeback and test coverage."}

7. Stay narrowly scoped:
   - Do not edit source files, update task.plan directly, or open PRs.
   - The only permitted mutation is writing `planning-duel/winner.json` via orbit.duel.plan.winner."#;

thread_local! {
    static TEST_PERMUTATION_QUEUE: RefCell<VecDeque<[usize; 3]>> =
        const { RefCell::new(VecDeque::new()) };
}

fn next_permutation<H: RuntimeHost + ?Sized>(host: &H) -> Result<[usize; 3], OrbitError> {
    let family_count = host.duel_candidate_families().len();
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
    if let Some(model) = host.duel_orchestrator_model(family) {
        return Ok(model);
    }
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
    let _ = orchestrator_model_for(host, family)?;
    Ok(PlanningRoleAssignment {
        family: AgentFamily::parse(family)?,
    })
}

// pub(crate) widened for tests/ layout under ORB-00225; test reaches via exposed surface.
pub(crate) fn build_roles_output<H: RuntimeHost + ?Sized>(
    host: &H,
    perm: [usize; 3],
) -> Result<Value, OrbitError> {
    let families = host.duel_candidate_families();
    let perm = validate_role_permutation(perm, families.len(), "select_planning_duel_roles")?;
    let planner_a = families[perm[0]].as_str();
    let planner_b = families[perm[1]].as_str();
    let arbiter = families[perm[2]].as_str();

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
            "orbit.graph.overview",
            "orbit.graph.search",
            "orbit.graph.show",
            "orbit.graph.refs",
            "orbit.graph.callees",
            "orbit.graph.impact",
            "orbit.graph.implementors",
            "orbit.graph.deps",
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
            "orbit.graph.overview",
            "orbit.graph.search",
            "orbit.graph.show",
            "orbit.graph.refs",
            "orbit.graph.callees",
            "orbit.graph.impact",
            "orbit.graph.implementors",
            "orbit.graph.deps",
            "fs.read",
        ],
    )
}

pub(super) fn planner_input_for_slot(task_id: &str, slot: RoleSlot) -> Value {
    json!({ "task_id": task_id, "planning_duel_slot": slot.as_str() })
}

pub(super) fn arbiter_input(task_id: &str) -> Value {
    json!({ "task_id": task_id, "planning_duel_slot": RoleSlot::Arbiter.as_str() })
}

pub(super) fn parse_planning_duel_roles(input: &Value) -> Result<PlanningRoles, OrbitError> {
    serde_json::from_value(input.get("planning_duel_roles").cloned().ok_or_else(|| {
        OrbitError::InvalidInput("missing required input.planning_duel_roles".to_string())
    })?)
    .map_err(|err| OrbitError::InvalidInput(format!("invalid planning_duel_roles payload: {err}")))
}

pub(super) fn select_planning_duel_roles<H: RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;

    let pa = input_string_field(input, "planner_a_family");
    let pb = input_string_field(input, "planner_b_family");
    let ar = input_string_field(input, "arbiter_family");

    let roles_output = if let (Some(a), Some(b), Some(c)) =
        (pa.as_deref(), pb.as_deref(), ar.as_deref())
    {
        // explicit assignment path (CLI or direct workflow); all-or-nothing already enforced by caller,
        // but defend here for partial YAML / direct activity calls
        if a == b || a == c || b == c {
            let dup = if a == b || a == c { a } else { b };
            return Err(OrbitError::InvalidInput(format!(
                "select_planning_duel_roles explicit roles must use distinct families; '{dup}' appears more than once"
            )));
        }

        let families = host.duel_candidate_families();
        let ia = families.iter().position(|f| f == a).ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "planner_a_family value '{a}' is not in [duel] candidates {families:?}"
            ))
        })?;
        let ib = families.iter().position(|f| f == b).ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "planner_b_family value '{b}' is not in [duel] candidates {families:?}"
            ))
        })?;
        let ic = families.iter().position(|f| f == c).ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "arbiter_family value '{c}' is not in [duel] candidates {families:?}"
            ))
        })?;

        let perm = [ia, ib, ic];
        validate_role_permutation(perm, families.len(), "select_planning_duel_roles")?;
        build_roles_output(host, perm)?
    } else if pa.is_some() || pb.is_some() || ar.is_some() {
        let mut missing = vec![];
        if pa.is_none() {
            missing.push("planner_a_family");
        }
        if pb.is_none() {
            missing.push("planner_b_family");
        }
        if ar.is_none() {
            missing.push("arbiter_family");
        }
        return Err(OrbitError::InvalidInput(format!(
            "select_planning_duel_roles explicit roles require all three of planner_a_family, planner_b_family, arbiter_family; missing {}",
            missing.join(", ")
        )));
    } else {
        let perm = next_permutation(host)?;
        build_roles_output(host, perm)?
    };

    Ok(json!({
        "task_id": task_id,
        "planning_duel_started_at": roles_output["planning_duel_started_at"].clone(),
        "planner_a_agent_cli": roles_output["planner_a_agent_cli"].clone(),
        "planner_a_model": roles_output["planner_a_model"].clone(),
        "planner_b_agent_cli": roles_output["planner_b_agent_cli"].clone(),
        "planner_b_model": roles_output["planner_b_model"].clone(),
        "arbiter_agent_cli": roles_output["arbiter_agent_cli"].clone(),
        "arbiter_model": roles_output["arbiter_model"].clone(),
        "planning_duel_roles": roles_output["planning_duel_roles"].clone(),
    }))
}
