//! `select_duel_roles` automation.
//!
//! Generates a random permutation of the three agent families
//! (`codex`, `claude`, `gemini`) across the three duel roles
//! (implementer, reviewer, arbiter) and writes them into the current
//! job input so downstream steps can resolve per-role agent CLIs via
//! `agent_cli_from_input` / `model_from_input`.
//!
//! The implementer is also stamped onto the task's internal routing
//! fields so that the reusable `implement_change` activity — which
//! resolves the agent from task metadata — picks up the implementer for
//! this run.
//!
//! Randomness source: `SystemTime` nanoseconds modulo the six possible
//! permutations. A thread-local test seam lets unit tests inject a
//! deterministic queue of permutation indices, so behavior can be
//! verified without patching the clock.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Utc;
use orbit_common::types::{OrbitError, all_agent_families};
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

use super::super::input::required_input_string;

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
                 update orbit_common::types::agent_pair::resolve_agent_model_pair"
            ))
        })
}

pub(in crate::executor::automation) fn select_duel_roles<H: RuntimeHost + TaskHost + ?Sized>(
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
