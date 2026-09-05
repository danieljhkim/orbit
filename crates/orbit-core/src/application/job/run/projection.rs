//! Shared JSON projection for persisted job runs.

use orbit_types::workflow::{JobRun, PipelineState};
use serde_json::{Value, json};

/// Project a job run and its optional persisted state for operator-facing APIs.
///
/// Child-dispatch lineage is historical and remains visible after a run reaches
/// a terminal state. Waiting reasons are momentary, so terminal runs omit them.
/// Presentation adapters may add intentionally surface-specific fields.
pub fn job_run_to_json(run: &JobRun, state: Option<&PipelineState>) -> Value {
    let last = run.steps.last();
    let child_dispatches = serde_json::to_value(
        state
            .map(|state| state.child_dispatches.as_slice())
            .unwrap_or_default(),
    )
    .unwrap_or_else(|_| Value::Array(Vec::new()));
    let state = (!run.state.is_terminal()).then_some(state).flatten();
    let waiting_on_deps = state
        .and_then(|state| state.waiting_on_deps.as_ref())
        .filter(|values| !values.is_empty());
    let waiting_on_locks = state
        .and_then(|state| state.waiting_on_locks.as_ref())
        .filter(|values| !values.is_empty());

    json!({
        "child_dispatches": child_dispatches,
        "run_id": run.run_id,
        "job_id": run.job_id,
        "attempt": run.attempt,
        "state": run.state.to_string(),
        "waiting_on_deps": waiting_on_deps,
        "waiting_on_locks": waiting_on_locks,
        "scheduled_at": run.scheduled_at.to_rfc3339(),
        "started_at": run.started_at.map(|value| value.to_rfc3339()),
        "finished_at": run.finished_at.map(|value| value.to_rfc3339()),
        "duration_ms": run.duration_ms,
        "retry_source_run_id": run.retry_source_run_id,
        "exit_code": last.and_then(|step| step.exit_code),
        "agent_response_json": last.and_then(|step| step.agent_response_json.as_ref()),
        "error_code": last.and_then(|step| step.error_code.as_deref()),
        "error_message": last.and_then(|step| step.error_message.as_deref()),
        "knowledge_metrics": run.knowledge_metrics,
        "resolved_crew": run.resolved_crew,
        "crew_model": run.crew_model,
        "steps": run.steps.iter().map(|step| json!({
            "step_index": step.step_index,
            "target_type": step.target_type.to_string(),
            "target_id": step.target_id,
            "state": step.state.to_string(),
            "started_at": step.started_at.map(|value| value.to_rfc3339()),
            "finished_at": step.finished_at.map(|value| value.to_rfc3339()),
            "duration_ms": step.duration_ms,
            "exit_code": step.exit_code,
            "agent_response_json": step.agent_response_json,
            "error_code": step.error_code,
            "error_message": step.error_message,
        })).collect::<Vec<_>>(),
        "created_at": run.created_at.to_rfc3339(),
    })
}
