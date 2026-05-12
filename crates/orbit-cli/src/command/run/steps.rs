use orbit_core::command::job::JobRunListParams;
use orbit_core::runtime::run_audit::RunAuditStep;
use orbit_core::{JobRun, JobRunStep, JobTargetType, NotFoundKind, OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use super::format::{format_duration, format_timestamp, summarize_error_message};

pub(crate) fn resolve_run(
    runtime: &OrbitRuntime,
    run_id: Option<&str>,
) -> Result<JobRun, OrbitError> {
    if let Some(run_id) = run_id {
        return runtime
            .show_job_run(run_id)
            .map_err(|_| OrbitError::not_found(NotFoundKind::JobRun, run_id.to_string()));
    }

    runtime
        .list_job_runs(JobRunListParams {
            limit: Some(1),
            ..Default::default()
        })?
        .into_iter()
        .next()
        .ok_or_else(|| OrbitError::not_found(NotFoundKind::JobRun, "latest".to_string()))
}

pub(crate) fn resolve_run_step(
    runtime: &OrbitRuntime,
    run: &JobRun,
    step_id: &str,
) -> Result<RunStepRecord, OrbitError> {
    if let Some(audit_step) = runtime
        .collect_run_audit_steps(&run.run_id)?
        .into_iter()
        .find(|step| step.step_id == step_id)
    {
        return Ok(RunStepRecord::from_audit_step(audit_step));
    }

    find_stored_run_step(run, step_id)
        .map(RunStepRecord::from_job_step)
        .ok_or_else(|| step_not_found(&run.run_id, step_id))
}

fn find_stored_run_step<'a>(run: &'a JobRun, step_id: &str) -> Option<&'a JobRunStep> {
    run.steps
        .iter()
        .find(|step| step.target_id == step_id || step.step_index.to_string() == step_id)
}

fn step_not_found(run_id: &str, step_id: &str) -> OrbitError {
    OrbitError::InvalidInput(format!(
        "step '{step_id}' does not match any step in run '{run_id}'"
    ))
}

pub(crate) fn filtered_steps<'a>(
    run: &'a JobRun,
    step_id: Option<&str>,
) -> Result<Vec<&'a JobRunStep>, OrbitError> {
    match step_id {
        Some(step_id) => Ok(vec![
            find_stored_run_step(run, step_id)
                .ok_or_else(|| step_not_found(&run.run_id, step_id))?,
        ]),
        None => Ok(run.steps.iter().collect()),
    }
}

pub(crate) fn resolve_step_filter(
    run: &JobRun,
    audit_steps: &[RunAuditStep],
    step_id: Option<&str>,
) -> Result<Option<String>, OrbitError> {
    let Some(step_id) = step_id else {
        return Ok(None);
    };

    if let Some(step) = audit_steps.iter().find(|step| step.step_id == step_id) {
        return Ok(Some(step.step_id.clone()));
    }
    if let Ok(index) = step_id.parse::<u32>()
        && let Some(step) = audit_steps.iter().find(|step| step.step_index == index)
    {
        return Ok(Some(step.step_id.clone()));
    }
    if find_stored_run_step(run, step_id).is_some() {
        return Ok(Some(step_id.to_string()));
    }

    Err(step_not_found(&run.run_id, step_id))
}

#[derive(Clone, Debug)]
pub(crate) struct RunStepRecord {
    pub(crate) step_index: u32,
    pub(crate) target_type: String,
    pub(crate) target_id: String,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    finished_at: Option<chrono::DateTime<chrono::Utc>>,
    duration_ms: Option<u64>,
    exit_code: Option<i32>,
    agent_response_json: Option<Value>,
    state: String,
    error_code: Option<String>,
    error_message: Option<String>,
}

impl RunStepRecord {
    fn from_job_step(step: &JobRunStep) -> Self {
        Self {
            step_index: step.step_index,
            target_type: step.target_type.to_string(),
            target_id: step.target_id.clone(),
            started_at: step.started_at,
            finished_at: step.finished_at,
            duration_ms: step.duration_ms,
            exit_code: step.exit_code,
            agent_response_json: step.agent_response_json.clone(),
            state: step.state.to_string(),
            error_code: step.error_code.clone(),
            error_message: step.error_message.clone(),
        }
    }

    fn from_audit_step(step: RunAuditStep) -> Self {
        let duration_ms = match (step.started_at, step.finished_at) {
            (Some(started), Some(finished)) => Some(
                finished
                    .signed_duration_since(started)
                    .num_milliseconds()
                    .max(0) as u64,
            ),
            _ => None,
        };
        Self {
            step_index: step.step_index,
            target_type: JobTargetType::Activity.to_string(),
            target_id: step.step_id,
            started_at: step.started_at,
            finished_at: step.finished_at,
            duration_ms,
            exit_code: None,
            agent_response_json: None,
            state: step.state.unwrap_or_else(|| "running".to_string()),
            error_code: None,
            error_message: step.error_message,
        }
    }
}

pub(crate) fn print_run_header(run: &JobRun) {
    use crate::output::color::{bold, dimmed, job_state_color};
    println!("{} {}", bold("Run ID:"), run.run_id);
    println!("{} {}", bold("Job ID:"), run.job_id);
    println!(
        "{} {}",
        bold("State:"),
        job_state_color(&run.state.to_string())
    );
    println!(
        "{} {}",
        bold("Started:"),
        dimmed(&format_timestamp(run.started_at))
    );
    println!(
        "{} {}",
        bold("Finished:"),
        dimmed(&format_timestamp(run.finished_at))
    );
    println!("{} {}", bold("Duration:"), format_duration(run.duration_ms));
}

pub(crate) fn print_step_summary_table(steps: &[&JobRunStep]) -> Result<(), OrbitError> {
    if steps.is_empty() {
        println!("No steps recorded.");
        return Ok(());
    }

    let mut table = crate::output::table::build_table(&[
        "#",
        "TARGET",
        "STATE",
        "DURATION",
        "ERROR CODE",
        "ERROR MESSAGE",
    ]);
    for step in steps {
        use comfy_table::Cell;
        table.add_row(vec![
            Cell::new(step.step_index),
            Cell::new(&step.target_id),
            crate::output::color::job_state_color_cell(&step.state.to_string()),
            Cell::new(format_duration(step.duration_ms)),
            Cell::new(step.error_code.as_deref().unwrap_or("-")),
            Cell::new(summarize_error_message(step.error_message.as_deref())),
        ]);
    }
    println!("{table}");
    Ok(())
}

pub(crate) fn print_step_record(
    run: &JobRun,
    step: &RunStepRecord,
    step_output: Option<Value>,
    json_output: bool,
) -> Result<(), OrbitError> {
    if json_output {
        return crate::output::json::print_pretty(&json!({
            "run_id": run.run_id,
            "job_id": run.job_id,
            "step": run_step_record_to_json(step),
            "step_output": step_output,
        }));
    }

    use crate::output::color::{bold, dimmed, job_state_color};
    println!("{} {}", bold("Run ID:"), run.run_id);
    println!("{} {}", bold("Job ID:"), run.job_id);
    println!("{} {}", bold("Target ID:"), step.target_id);
    println!("{} {}", bold("Target Type:"), step.target_type);
    println!("{} {}", bold("State:"), job_state_color(&step.state));
    println!(
        "{} {}",
        bold("Started:"),
        dimmed(&format_timestamp(step.started_at))
    );
    println!(
        "{} {}",
        bold("Finished:"),
        dimmed(&format_timestamp(step.finished_at))
    );
    println!(
        "{} {}",
        bold("Duration:"),
        format_duration(step.duration_ms)
    );
    println!(
        "{} {}",
        bold("Exit Code:"),
        step.exit_code
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!(
        "{} {}",
        bold("Error Code:"),
        step.error_code.as_deref().unwrap_or("-")
    );
    println!(
        "{} {}",
        bold("Error Message:"),
        step.error_message.as_deref().unwrap_or("-")
    );
    if let Some(output) = step_output {
        println!("{}", bold("Step Output:"));
        println!(
            "{}",
            serde_json::to_string_pretty(&output)
                .map_err(|err| OrbitError::Store(err.to_string()))?
        );
    }
    Ok(())
}

fn run_step_record_to_json(step: &RunStepRecord) -> Value {
    json!({
        "step_index": step.step_index,
        "target_id": step.target_id,
        "target_type": step.target_type,
        "state": step.state,
        "started_at": step.started_at.map(|t| t.to_rfc3339()),
        "finished_at": step.finished_at.map(|t| t.to_rfc3339()),
        "duration_ms": step.duration_ms,
        "exit_code": step.exit_code,
        "agent_response_json": step.agent_response_json,
        "error_code": step.error_code,
        "error_message": step.error_message,
    })
}

pub(crate) fn legacy_step_to_json(step: &JobRunStep) -> Value {
    json!({
        "step_index": step.step_index,
        "target_id": step.target_id,
        "target_type": step.target_type.to_string(),
        "state": step.state.to_string(),
        "started_at": step.started_at.map(|t| t.to_rfc3339()),
        "finished_at": step.finished_at.map(|t| t.to_rfc3339()),
        "duration_ms": step.duration_ms,
        "exit_code": step.exit_code,
        "error_code": step.error_code,
        "error_message": step.error_message,
    })
}
