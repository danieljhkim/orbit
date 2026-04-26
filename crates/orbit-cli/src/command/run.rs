use std::collections::{HashMap, HashSet};

use clap::{Args, Subcommand};
use orbit_core::command::job_run::JobRunListParams;
use orbit_core::runtime::run_audit::{RunAuditEvent, RunAuditStep, RunCliInvocationRecord};
use orbit_core::{JobRun, JobRunStep, JobTargetType, OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;
use crate::command::job::{job_run_to_json, summarize_error_message};
use crate::command::{duel, job, ship};

const DEFAULT_HISTORY_LIMIT: usize = 50;

const RUN_AFTER_HELP: &str = "\
Workflow entrypoints:
  orbit run ship <task_id> ...
  orbit run ship-auto
  orbit run duel-plan <task_id>
  orbit run job <job_id> [--input key=value] [--json] [--debug]

Run history:
  orbit run history [--limit 50]
  orbit run history -j <job_id>
  orbit run show [run_id] [-s step_id] [--json]
  orbit run logs [run_id] [-s step_id] [--json]
  orbit run events [run_id] [-s step_id] [--type event_type] [--json]
  orbit run trace [run_id] [--json]
";

#[derive(Args)]
#[command(
    about = "Run a job workflow (supports run ship / ship-auto / duel-plan / job)",
    arg_required_else_help = true,
    subcommand_required = true,
    override_usage = "orbit run <COMMAND>",
    after_help = RUN_AFTER_HELP,
    help_template = "\
{about}

{usage-heading} {usage}

Workflows:
  ship       Ship explicitly selected tasks through the task pipeline
  ship-auto  Auto-select backlog tasks and ship them through the task pipeline
  duel-plan  Run a planning duel for one task
  job        Run an arbitrary job by ID

Audits:
  history    Show recent job runs, optionally filtered to one job
  show       Show structured state and step summary for a job run
  logs       Print raw stdout/stderr captured for a job run
  events     Show audit events recorded for a job run
  trace      Show audit event parent/child trace for a job run

Options:
{options}
{after-help}"
)]
pub struct RunCommand {
    #[command(subcommand)]
    pub command: RunSubcommand,
}

impl Execute for RunCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum RunSubcommand {
    /// Ship explicitly selected tasks through the task pipeline
    Ship(ship::ShipCommand),
    /// Auto-select backlog tasks and ship them through the task pipeline
    #[command(name = "ship-auto")]
    ShipAuto(ship::ShipAutoCommand),
    /// Run a planning duel for one task
    #[command(name = "duel-plan")]
    DuelPlan(duel::DuelPlanCommand),
    /// Show recent job runs, optionally filtered to one job
    History(RunHistoryArgs),
    /// Show structured state and step summary for a job run
    Show(RunShowArgs),
    /// Print raw stdout/stderr captured for a job run
    Logs(RunLogsArgs),
    /// Show audit events recorded for a job run
    Events(RunEventsArgs),
    /// Show audit event parent/child trace for a job run
    Trace(RunTraceArgs),
    /// Run an arbitrary job by ID
    Job(job::JobRunArgs),
}

impl Execute for RunSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            RunSubcommand::Ship(command) => command.execute(runtime),
            RunSubcommand::ShipAuto(command) => command.execute(runtime),
            RunSubcommand::DuelPlan(command) => command.execute(runtime),
            RunSubcommand::History(command) => command.execute(runtime),
            RunSubcommand::Show(command) => command.execute(runtime),
            RunSubcommand::Logs(command) => command.execute(runtime),
            RunSubcommand::Events(command) => command.execute(runtime),
            RunSubcommand::Trace(command) => command.execute(runtime),
            RunSubcommand::Job(command) => command.execute(runtime),
        }
    }
}

#[derive(Args)]
#[command(
    after_help = "JSON shape: {\"runs\":[<job-run>]}\nExamples:\n  orbit run history\n  orbit run history -j task_local_pipeline --limit 20\n  orbit run history --json"
)]
pub struct RunHistoryArgs {
    /// Filter to one job ID
    #[arg(short = 'j', long = "job")]
    pub job_id: Option<String>,

    /// Maximum number of runs to show
    #[arg(long, default_value_t = DEFAULT_HISTORY_LIMIT)]
    pub limit: usize,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for RunHistoryArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        print_run_history(runtime, self.job_id.as_deref(), Some(self.limit), self.json)
    }
}

#[derive(Args)]
#[command(
    after_help = "JSON shape: {\"run\":<job-run>,\"pipeline_state\":<state|null>} or {\"run_id\":...,\"job_id\":...,\"step\":<step>,\"step_output\":<json|null>} with -s.\nExamples:\n  orbit run show\n  orbit run show jrun-20260426-0631\n  orbit run show jrun-20260426-0631 -s implement_one --json"
)]
pub struct RunShowArgs {
    /// Run ID to inspect. Defaults to the most recently scheduled run globally.
    pub run_id: Option<String>,

    /// Show a single activity step.id from the v2 job YAML; legacy target ID and index still work
    #[arg(short = 's', long = "step")]
    pub step_id: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for RunShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        print_run_show(
            runtime,
            self.run_id.as_deref(),
            self.step_id.as_deref(),
            self.json,
        )
    }
}

#[derive(Args)]
#[command(
    after_help = "JSON shape: {\"run_id\":\"...\",\"job_id\":\"...\",\"records\":[{\"step_id\":...,\"stdout_blob_ref\":...,\"stderr_blob_ref\":...,\"stdout\":\"...\",\"stderr\":\"...\"}]}\nExamples:\n  orbit run logs\n  orbit run logs jrun-20260426-0631\n  orbit run logs jrun-20260426-0631 -s implement_one --json"
)]
pub struct RunLogsArgs {
    /// Run ID to inspect. Defaults to the most recently scheduled run globally.
    pub run_id: Option<String>,

    /// Show raw logs for a single activity step.id from the v2 job YAML
    #[arg(short = 's', long = "step")]
    pub step_id: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for RunLogsArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        print_run_logs(
            runtime,
            self.run_id.as_deref(),
            self.step_id.as_deref(),
            self.json,
        )
    }
}

#[derive(Args)]
#[command(
    after_help = "JSON shape: {\"run_id\":\"...\",\"job_id\":\"...\",\"events\":[<raw-v2-audit-event-with-step_id>]}\nExamples:\n  orbit run events\n  orbit run events jrun-20260426-0631\n  orbit run events jrun-20260426-0631 -s implement_one --type cli.invocation.finished --json"
)]
pub struct RunEventsArgs {
    /// Run ID to inspect. Defaults to the most recently scheduled run globally.
    pub run_id: Option<String>,

    /// Filter to an activity step.id from the v2 job YAML, or its zero-based audit step index
    #[arg(short = 's', long = "step")]
    pub step_id: Option<String>,

    /// Filter to an exact v2 audit event_type such as step.started
    #[arg(long = "type")]
    pub event_type: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for RunEventsArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        print_run_events(
            runtime,
            self.run_id.as_deref(),
            self.step_id.as_deref(),
            self.event_type.as_deref(),
            self.json,
        )
    }
}

#[derive(Args)]
#[command(
    after_help = "JSON shape: {\"run_id\":\"...\",\"job_id\":\"...\",\"roots\":[<tree-node>],\"orphans\":[<tree-node>]}\nExamples:\n  orbit run trace\n  orbit run trace jrun-20260426-0631 --json"
)]
pub struct RunTraceArgs {
    /// Run ID to inspect. Defaults to the most recently scheduled run globally.
    pub run_id: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for RunTraceArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        print_run_trace(runtime, self.run_id.as_deref(), self.json)
    }
}

pub(crate) fn print_run_history(
    runtime: &OrbitRuntime,
    job_id: Option<&str>,
    limit: Option<usize>,
    json_output: bool,
) -> Result<(), OrbitError> {
    let runs = match job_id {
        Some(job_id) => runtime.list_job_runs(JobRunListParams {
            job_id: Some(job_id.to_string()),
            limit,
            ..Default::default()
        })?,
        None => runtime.list_job_runs(JobRunListParams {
            limit,
            ..Default::default()
        })?,
    };

    if json_output {
        let values = runs.iter().map(job_run_to_json).collect::<Vec<_>>();
        return crate::output::json::print_pretty(&json!({ "runs": values }));
    }

    let include_job_id = job_id.is_none();
    let headers = if include_job_id {
        vec![
            "RUN_ID",
            "JOB_ID",
            "ATTEMPT",
            "STATE",
            "STARTED_AT",
            "FINISHED_AT",
            "ERROR_CODE",
            "ERROR_MESSAGE",
        ]
    } else {
        vec![
            "RUN_ID",
            "ATTEMPT",
            "STATE",
            "STARTED_AT",
            "FINISHED_AT",
            "ERROR_CODE",
            "ERROR_MESSAGE",
        ]
    };
    let mut table = crate::output::table::build_table(&headers);
    for run in &runs {
        use comfy_table::Cell;
        let last = run.steps.last();
        let mut row = vec![Cell::new(&run.run_id)];
        if include_job_id {
            row.push(Cell::new(&run.job_id));
        }
        row.extend([
            Cell::new(run.attempt.to_string()),
            crate::output::color::job_state_color_cell(&run.state.to_string()),
            Cell::new(format_timestamp(run.started_at)),
            Cell::new(format_timestamp(run.finished_at)),
            Cell::new(last.and_then(|s| s.error_code.as_deref()).unwrap_or("-")),
            Cell::new(summarize_error_message(
                last.and_then(|s| s.error_message.as_deref()),
            )),
        ]);
        table.add_row(row);
    }
    println!("{table}");
    Ok(())
}

pub(crate) fn print_run_show(
    runtime: &OrbitRuntime,
    run_id: Option<&str>,
    step_id: Option<&str>,
    json_output: bool,
) -> Result<(), OrbitError> {
    let run = resolve_run(runtime, run_id)?;
    let state = runtime.read_run_state(&run.run_id)?;

    if let Some(step_id) = step_id {
        let step = resolve_run_step(runtime, &run, step_id)?;
        let step_output = state
            .as_ref()
            .and_then(|state| state.step_outputs.get(&step.step_index))
            .cloned();
        return print_step_record(&run, &step, step_output, json_output);
    }

    if json_output {
        return crate::output::json::print_pretty(&json!({
            "run": job_run_to_json(&run),
            "pipeline_state": state,
        }));
    }

    print_run_header(&run);
    if let Some(state) = &state {
        println!(
            "{} iteration={} step_outputs={} updated_at={}",
            crate::output::color::bold("Pipeline:"),
            state.iteration,
            state.step_outputs.len(),
            state.updated_at.to_rfc3339(),
        );
    }
    println!();
    let steps = run.steps.iter().collect::<Vec<_>>();
    print_step_summary_table(&steps)
}

pub(crate) fn print_run_logs(
    runtime: &OrbitRuntime,
    run_id: Option<&str>,
    step_id: Option<&str>,
    json_output: bool,
) -> Result<(), OrbitError> {
    let run = resolve_run(runtime, run_id)?;
    let audit_steps = runtime.collect_run_audit_steps(&run.run_id)?;
    let step_filter = resolve_step_filter(&run, &audit_steps, step_id)?;
    let records = filter_cli_invocation_records(
        runtime.collect_run_cli_invocations(&run.run_id)?,
        step_filter.as_deref(),
    );

    if json_output {
        return crate::output::json::print_pretty(&json!({
            "run_id": run.run_id,
            "job_id": run.job_id,
            "records": records.iter().map(cli_invocation_record_to_json).collect::<Vec<_>>(),
        }));
    }

    if records.is_empty() {
        println!("No raw stdout/stderr blobs recorded.");
        return Ok(());
    }

    for record in &records {
        print!("{}", record.stdout);
        eprint!("{}", record.stderr);
    }
    Ok(())
}

pub(crate) fn print_run_events(
    runtime: &OrbitRuntime,
    run_id: Option<&str>,
    step_id: Option<&str>,
    event_type: Option<&str>,
    json_output: bool,
) -> Result<(), OrbitError> {
    let run = resolve_run(runtime, run_id)?;
    let audit_steps = runtime.collect_run_audit_steps(&run.run_id)?;
    let step_filter = resolve_step_filter(&run, &audit_steps, step_id)?;
    let events = filter_run_audit_events(
        runtime.collect_run_audit_events(&run.run_id)?,
        step_filter.as_deref(),
        event_type,
    );

    if json_output {
        return crate::output::json::print_pretty(&json!({
            "run_id": run.run_id,
            "job_id": run.job_id,
            "events": events.iter().map(RunAuditEvent::json_with_step_id).collect::<Vec<_>>(),
        }));
    }

    if events.is_empty() {
        println!("No audit events recorded.");
        return Ok(());
    }

    let mut table = crate::output::table::build_table(&["TS", "STEP", "EVENT_TYPE", "SUMMARY"]);
    for event in &events {
        use comfy_table::Cell;
        table.add_row(vec![
            Cell::new(format_timestamp(event.timestamp)),
            Cell::new(event.step_id.as_deref().unwrap_or("-")),
            Cell::new(event.event_type.as_deref().unwrap_or("-")),
            Cell::new(summarize_audit_event(event)),
        ]);
    }
    println!("{table}");
    Ok(())
}

pub(crate) fn print_run_trace(
    runtime: &OrbitRuntime,
    run_id: Option<&str>,
    json_output: bool,
) -> Result<(), OrbitError> {
    let run = resolve_run(runtime, run_id)?;
    let events = runtime.collect_run_audit_events(&run.run_id)?;
    let tree = build_trace_tree(&events);

    if json_output {
        return crate::output::json::print_pretty(&json!({
            "run_id": run.run_id,
            "job_id": run.job_id,
            "roots": tree.roots.iter().map(trace_node_to_json).collect::<Vec<_>>(),
            "orphans": tree.orphans.iter().map(trace_node_to_json).collect::<Vec<_>>(),
        }));
    }

    if tree.roots.is_empty() && tree.orphans.is_empty() {
        println!("No audit events recorded.");
        return Ok(());
    }

    for node in &tree.roots {
        print_trace_node(node, 0);
    }
    if !tree.orphans.is_empty() {
        println!("Orphans:");
        for node in &tree.orphans {
            print_trace_node(node, 1);
        }
    }
    Ok(())
}

pub(crate) fn print_legacy_logs_summary(
    runtime: &OrbitRuntime,
    run_id: &str,
    step_id: Option<&str>,
    json_output: bool,
) -> Result<(), OrbitError> {
    let run = runtime
        .show_job_run(run_id)
        .map_err(|_| OrbitError::JobRunNotFound(run_id.to_string()))?;
    let steps = filtered_steps(&run, step_id)?;

    if json_output {
        let values = steps
            .iter()
            .map(|step| legacy_step_to_json(step))
            .collect::<Vec<_>>();
        return crate::output::json::print_pretty(&Value::Array(values));
    }

    print_run_header(&run);
    println!();
    print_step_summary_table(&steps)
}

fn resolve_run(runtime: &OrbitRuntime, run_id: Option<&str>) -> Result<JobRun, OrbitError> {
    if let Some(run_id) = run_id {
        return runtime
            .show_job_run(run_id)
            .map_err(|_| OrbitError::JobRunNotFound(run_id.to_string()));
    }

    runtime
        .list_job_runs(JobRunListParams {
            limit: Some(1),
            ..Default::default()
        })?
        .into_iter()
        .next()
        .ok_or_else(|| OrbitError::JobRunNotFound("latest".to_string()))
}

fn resolve_run_step(
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

fn filtered_steps<'a>(
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

fn resolve_step_filter(
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
struct RunStepRecord {
    step_index: u32,
    target_type: String,
    target_id: String,
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

fn print_run_header(run: &JobRun) {
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

fn print_step_summary_table(steps: &[&JobRunStep]) -> Result<(), OrbitError> {
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

fn print_step_record(
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

fn legacy_step_to_json(step: &JobRunStep) -> Value {
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

fn filter_cli_invocation_records(
    records: Vec<RunCliInvocationRecord>,
    step_filter: Option<&str>,
) -> Vec<RunCliInvocationRecord> {
    records
        .into_iter()
        .filter(|record| step_filter.is_none_or(|filter| record.step_id.as_deref() == Some(filter)))
        .collect()
}

fn cli_invocation_record_to_json(record: &RunCliInvocationRecord) -> Value {
    json!({
        "step_id": record.step_id,
        "provider": record.provider,
        "stdout_blob_ref": record.stdout_blob_ref,
        "stderr_blob_ref": record.stderr_blob_ref,
        "stdout": record.stdout,
        "stderr": record.stderr,
    })
}

fn filter_run_audit_events(
    events: Vec<RunAuditEvent>,
    step_filter: Option<&str>,
    event_type: Option<&str>,
) -> Vec<RunAuditEvent> {
    events
        .into_iter()
        .filter(|event| {
            step_filter.is_none_or(|filter| event.step_id.as_deref() == Some(filter))
                && event_type.is_none_or(|filter| event.event_type.as_deref() == Some(filter))
        })
        .collect()
}

fn summarize_audit_event(event: &RunAuditEvent) -> String {
    let raw = &event.raw;
    match event.event_type.as_deref() {
        Some("run.started") => field_summary(raw, "job_name"),
        Some("run.finished") => field_summary(raw, "outcome"),
        Some("step.started") => field_summary(raw, "step_id"),
        Some("step.finished") => join_present(&[
            ("step", raw_str(raw, "step_id")),
            ("outcome", raw_str(raw, "outcome")),
        ]),
        Some("step.skipped") => join_present(&[
            ("step", raw_str(raw, "step_id")),
            ("reason", raw_str(raw, "reason")),
        ]),
        Some("step.denied") => join_present(&[
            ("step", raw_str(raw, "step_id")),
            ("reason", raw_str(raw, "reason")),
        ]),
        Some("activity.started") => join_present(&[
            ("activity", raw_str(raw, "activity_name")),
            ("type", raw_str(raw, "activity_type")),
        ]),
        Some("activity.finished") => join_present(&[
            ("activity", raw_str(raw, "activity_name")),
            ("outcome", raw_str(raw, "outcome")),
        ]),
        Some("cli.invocation.started") => join_present(&[
            ("provider", raw_str(raw, "provider")),
            ("model", raw_str(raw, "model")),
        ]),
        Some("cli.invocation.finished") => join_present(&[
            ("provider", raw_str(raw, "provider")),
            ("exit", raw_i64(raw, "exit_code")),
            ("duration_ms", raw_u64(raw, "duration_ms")),
            ("timed_out", raw_bool(raw, "timed_out")),
        ]),
        Some("tool.denied") => join_present(&[
            ("tool", raw_str(raw, "tool_name")),
            ("reason", raw_str(raw, "reason")),
        ]),
        Some("fs.call.request" | "fs.call.result" | "fs.call.denied") => join_present(&[
            ("op", raw_str(raw, "op")),
            ("path", raw_str(raw, "path")),
            ("allowed", raw_bool(raw, "allowed")),
        ]),
        Some("fanout.dispatched") => join_present(&[
            ("step", raw_str(raw, "step_id")),
            ("workers", raw_u64(raw, "worker_count")),
        ]),
        Some("worker.state") => join_present(&[
            ("step", raw_str(raw, "step_id")),
            ("worker", raw_u64(raw, "worker_index")),
            ("state", raw_str(raw, "state")),
        ]),
        Some("fanin.joined") => join_present(&[
            ("step", raw_str(raw, "step_id")),
            ("collected", raw_u64(raw, "collected")),
            ("failed", raw_u64(raw, "failed")),
        ]),
        _ => event.body_kind.clone().unwrap_or_else(|| "-".to_string()),
    }
}

fn field_summary(raw: &Value, field: &str) -> String {
    raw_str(raw, field).unwrap_or_else(|| "-".to_string())
}

fn raw_str(raw: &Value, field: &str) -> Option<String> {
    raw.get(field).and_then(Value::as_str).map(str::to_string)
}

fn raw_i64(raw: &Value, field: &str) -> Option<String> {
    raw.get(field)
        .and_then(Value::as_i64)
        .map(|value| value.to_string())
}

fn raw_u64(raw: &Value, field: &str) -> Option<String> {
    raw.get(field)
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
}

fn raw_bool(raw: &Value, field: &str) -> Option<String> {
    raw.get(field)
        .and_then(Value::as_bool)
        .map(|value| value.to_string())
}

fn join_present(fields: &[(&str, Option<String>)]) -> String {
    let summary = fields
        .iter()
        .filter_map(|(label, value)| value.as_ref().map(|value| format!("{label}={value}")))
        .collect::<Vec<_>>()
        .join(" ");
    if summary.is_empty() {
        "-".to_string()
    } else {
        summary
    }
}

#[derive(Clone, Debug)]
struct TraceTree {
    roots: Vec<TraceNode>,
    orphans: Vec<TraceNode>,
}

#[derive(Clone, Debug)]
struct TraceNode {
    event: RunAuditEvent,
    children: Vec<TraceNode>,
}

fn build_trace_tree(events: &[RunAuditEvent]) -> TraceTree {
    let index_by_id = events
        .iter()
        .enumerate()
        .map(|(index, event)| (event.event_id.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut child_indexes = vec![Vec::<usize>::new(); events.len()];
    let mut roots = Vec::new();
    let mut orphans = Vec::new();

    for (index, event) in events.iter().enumerate() {
        match event
            .parent_event_id
            .as_ref()
            .and_then(|parent_id| index_by_id.get(parent_id))
        {
            Some(parent_index) => child_indexes[*parent_index].push(index),
            None if event.parent_event_id.is_some() => orphans.push(index),
            None => roots.push(index),
        }
    }

    TraceTree {
        roots: roots
            .into_iter()
            .map(|index| build_trace_node(index, events, &child_indexes, &mut HashSet::new()))
            .collect(),
        orphans: orphans
            .into_iter()
            .map(|index| build_trace_node(index, events, &child_indexes, &mut HashSet::new()))
            .collect(),
    }
}

fn build_trace_node(
    index: usize,
    events: &[RunAuditEvent],
    child_indexes: &[Vec<usize>],
    visited: &mut HashSet<usize>,
) -> TraceNode {
    if !visited.insert(index) {
        return TraceNode {
            event: events[index].clone(),
            children: Vec::new(),
        };
    }
    let children = child_indexes[index]
        .iter()
        .map(|child_index| build_trace_node(*child_index, events, child_indexes, visited))
        .collect::<Vec<_>>();
    visited.remove(&index);
    TraceNode {
        event: events[index].clone(),
        children,
    }
}

fn trace_node_to_json(node: &TraceNode) -> Value {
    json!({
        "event": node.event.json_with_step_id(),
        "children": node.children.iter().map(trace_node_to_json).collect::<Vec<_>>(),
    })
}

fn print_trace_node(node: &TraceNode, depth: usize) {
    let indent = "  ".repeat(depth);
    let prefix = if depth == 0 { "" } else { "- " };
    println!(
        "{indent}{prefix}{} {}",
        node.event.event_type.as_deref().unwrap_or("-"),
        summarize_audit_event(&node.event)
    );
    for child in &node.children {
        print_trace_node(child, depth + 1);
    }
}

fn format_timestamp(value: Option<chrono::DateTime<chrono::Utc>>) -> String {
    value
        .map(|v| v.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn format_duration(value: Option<u64>) -> String {
    value
        .map(|duration| format!("{duration}ms"))
        .unwrap_or_else(|| "-".to_string())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::command::{Cli, Commands};

    use super::*;

    fn parse_run(args: &[&str]) -> RunCommand {
        let cli = Cli::parse_from(args);
        match cli.command {
            Commands::Run(command) => command,
            _ => panic!("expected run command"),
        }
    }

    #[test]
    fn parses_explicit_ship_defaults() {
        let command = parse_run(&["orbit", "run", "ship", "T1", "T2"]);
        match command.command {
            RunSubcommand::Ship(args) => {
                assert_eq!(args.task_ids, vec!["T1", "T2"]);
                assert_eq!(args.mode, ship::ShipMode::Pr);
                assert_eq!(args.base, "agent-main");
            }
            _ => panic!("expected ship"),
        }
    }

    #[test]
    fn parses_explicit_ship_mode_and_base() {
        let command = parse_run(&["orbit", "run", "ship", "-m", "local", "-b", "main", "T1"]);
        match command.command {
            RunSubcommand::Ship(args) => {
                assert_eq!(args.task_ids, vec!["T1"]);
                assert_eq!(args.mode, ship::ShipMode::Local);
                assert_eq!(args.base, "main");
            }
            _ => panic!("expected ship"),
        }
    }

    #[test]
    fn parses_ship_auto_as_top_level_subcommand() {
        let command = parse_run(&["orbit", "run", "ship-auto", "-m", "pr", "-b", "main"]);
        match command.command {
            RunSubcommand::ShipAuto(args) => {
                assert_eq!(args.mode, ship::ShipMode::Pr);
                assert_eq!(args.base, "main");
            }
            _ => panic!("expected ship-auto"),
        }
    }

    #[test]
    fn parses_duel_plan_as_top_level_subcommand() {
        let command = parse_run(&["orbit", "run", "duel-plan", "T1", "-b", "main"]);
        match command.command {
            RunSubcommand::DuelPlan(args) => {
                assert_eq!(args.task_id, "T1");
                assert_eq!(args.base, "main");
            }
            _ => panic!("expected duel-plan"),
        }
    }

    #[test]
    fn parses_run_job_unchanged() {
        let command = parse_run(&["orbit", "run", "job", "task_auto_pipeline", "--json"]);
        match command.command {
            RunSubcommand::Job(args) => {
                assert_eq!(args.job_id, "task_auto_pipeline");
                assert!(args.json);
            }
            _ => panic!("expected job"),
        }
    }

    #[test]
    fn rejects_positional_job_fallback() {
        assert!(Cli::try_parse_from(["orbit", "run", "task_auto_pipeline", "--json"]).is_err());
    }

    #[test]
    fn parses_run_history_defaults() {
        let command = parse_run(&["orbit", "run", "history"]);
        match command.command {
            RunSubcommand::History(args) => {
                assert_eq!(args.job_id, None);
                assert_eq!(args.limit, DEFAULT_HISTORY_LIMIT);
                assert!(!args.json);
            }
            _ => panic!("expected history"),
        }
    }

    #[test]
    fn parses_run_history_job_filter() {
        let command = parse_run(&["orbit", "run", "history", "-j", "task_auto_pipeline"]);
        match command.command {
            RunSubcommand::History(args) => {
                assert_eq!(args.job_id.as_deref(), Some("task_auto_pipeline"));
                assert_eq!(args.limit, DEFAULT_HISTORY_LIMIT);
            }
            _ => panic!("expected history"),
        }
    }

    #[test]
    fn parses_run_show_latest() {
        let command = parse_run(&["orbit", "run", "show"]);
        match command.command {
            RunSubcommand::Show(args) => {
                assert_eq!(args.run_id, None);
                assert_eq!(args.step_id, None);
            }
            _ => panic!("expected show"),
        }
    }

    #[test]
    fn parses_run_show_run_id() {
        let command = parse_run(&["orbit", "run", "show", "jrun-1"]);
        match command.command {
            RunSubcommand::Show(args) => {
                assert_eq!(args.run_id.as_deref(), Some("jrun-1"));
                assert_eq!(args.step_id, None);
            }
            _ => panic!("expected show"),
        }
    }

    #[test]
    fn parses_run_show_step() {
        let command = parse_run(&["orbit", "run", "show", "jrun-1", "-s", "implement_one"]);
        match command.command {
            RunSubcommand::Show(args) => {
                assert_eq!(args.run_id.as_deref(), Some("jrun-1"));
                assert_eq!(args.step_id.as_deref(), Some("implement_one"));
            }
            _ => panic!("expected show"),
        }
    }

    #[test]
    fn parses_run_logs_latest() {
        let command = parse_run(&["orbit", "run", "logs"]);
        match command.command {
            RunSubcommand::Logs(args) => {
                assert_eq!(args.run_id, None);
                assert_eq!(args.step_id, None);
            }
            _ => panic!("expected logs"),
        }
    }

    #[test]
    fn parses_run_logs_run_id() {
        let command = parse_run(&["orbit", "run", "logs", "jrun-1"]);
        match command.command {
            RunSubcommand::Logs(args) => {
                assert_eq!(args.run_id.as_deref(), Some("jrun-1"));
                assert_eq!(args.step_id, None);
            }
            _ => panic!("expected logs"),
        }
    }

    #[test]
    fn parses_run_logs_step() {
        let command = parse_run(&["orbit", "run", "logs", "jrun-1", "-s", "implement_one"]);
        match command.command {
            RunSubcommand::Logs(args) => {
                assert_eq!(args.run_id.as_deref(), Some("jrun-1"));
                assert_eq!(args.step_id.as_deref(), Some("implement_one"));
            }
            _ => panic!("expected logs"),
        }
    }

    #[test]
    fn parses_run_events_latest() {
        let command = parse_run(&["orbit", "run", "events"]);
        match command.command {
            RunSubcommand::Events(args) => {
                assert_eq!(args.run_id, None);
                assert_eq!(args.step_id, None);
                assert_eq!(args.event_type, None);
                assert!(!args.json);
            }
            _ => panic!("expected events"),
        }
    }

    #[test]
    fn parses_run_events_filters() {
        let command = parse_run(&[
            "orbit",
            "run",
            "events",
            "jrun-1",
            "-s",
            "implement_one",
            "--type",
            "cli.invocation.finished",
            "--json",
        ]);
        match command.command {
            RunSubcommand::Events(args) => {
                assert_eq!(args.run_id.as_deref(), Some("jrun-1"));
                assert_eq!(args.step_id.as_deref(), Some("implement_one"));
                assert_eq!(args.event_type.as_deref(), Some("cli.invocation.finished"));
                assert!(args.json);
            }
            _ => panic!("expected events"),
        }
    }

    #[test]
    fn parses_run_trace_latest() {
        let command = parse_run(&["orbit", "run", "trace"]);
        match command.command {
            RunSubcommand::Trace(args) => {
                assert_eq!(args.run_id, None);
                assert!(!args.json);
            }
            _ => panic!("expected trace"),
        }
    }

    #[test]
    fn parses_run_trace_json() {
        let command = parse_run(&["orbit", "run", "trace", "jrun-1", "--json"]);
        match command.command {
            RunSubcommand::Trace(args) => {
                assert_eq!(args.run_id.as_deref(), Some("jrun-1"));
                assert!(args.json);
            }
            _ => panic!("expected trace"),
        }
    }

    #[test]
    fn run_events_filter_by_step_and_type() {
        let events = vec![
            test_audit_event("evt-run", None, "run.started", None),
            test_audit_event(
                "evt-step",
                Some("evt-run"),
                "step.started",
                Some("implement_one"),
            ),
            test_audit_event(
                "evt-cli",
                Some("evt-step"),
                "cli.invocation.finished",
                Some("implement_one"),
            ),
            test_audit_event(
                "evt-review",
                Some("evt-run"),
                "step.started",
                Some("review"),
            ),
        ];

        let filtered = filter_run_audit_events(
            events,
            Some("implement_one"),
            Some("cli.invocation.finished"),
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].event_id, "evt-cli");
    }

    #[test]
    fn run_trace_tree_nests_children_and_keeps_orphans() {
        let events = vec![
            test_audit_event("evt-run", None, "run.started", None),
            test_audit_event(
                "evt-step",
                Some("evt-run"),
                "step.started",
                Some("implement_one"),
            ),
            test_audit_event(
                "evt-activity",
                Some("evt-step"),
                "activity.started",
                Some("implement_one"),
            ),
            test_audit_event("evt-orphan", Some("evt-missing"), "tool.denied", None),
        ];

        let tree = build_trace_tree(&events);
        assert_eq!(tree.roots.len(), 1);
        assert_eq!(tree.roots[0].event.event_id, "evt-run");
        assert_eq!(tree.roots[0].children[0].event.event_id, "evt-step");
        assert_eq!(
            tree.roots[0].children[0].children[0].event.event_id,
            "evt-activity"
        );
        assert_eq!(tree.orphans.len(), 1);
        assert_eq!(tree.orphans[0].event.event_id, "evt-orphan");
    }

    #[test]
    fn resolve_run_step_prefers_audit_step_id() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let yaml_path = runtime.data_root().join("qa_step_id.yaml");
        std::fs::write(
            &yaml_path,
            r#"schemaVersion: 2
kind: Job
metadata:
  name: qa_step_id
spec:
  state: enabled
  kind: workflow
  steps:
    - id: nap
      spec:
        type: deterministic
        action: sleep
        config: {}
"#,
        )
        .expect("write job yaml");
        let result = runtime
            .run_job_v2_from_yaml(&yaml_path, json!({ "seconds": 0 }), None)
            .expect("run job");
        let run = runtime.show_job_run(&result.run_id).expect("show run");

        let resolved = resolve_run_step(&runtime, &run, "nap").expect("resolve step");
        assert_eq!(resolved.target_id, "nap");
        assert_eq!(resolved.target_type, "activity");
    }

    #[test]
    fn rejects_removed_duel_history_forms() {
        assert!(Cli::try_parse_from(["orbit", "run", "duel", "list"]).is_err());
        assert!(Cli::try_parse_from(["orbit", "run", "duel", "show"]).is_err());
    }

    fn test_audit_event(
        event_id: &str,
        parent_event_id: Option<&str>,
        event_type: &str,
        step_id: Option<&str>,
    ) -> RunAuditEvent {
        let body_kind = event_type.replace('.', "_");
        let mut raw = json!({
            "schemaVersion": 1,
            "event_type": event_type,
            "event_id": event_id,
            "ts": "2026-04-26T07:00:00Z",
            "run_id": "jrun-test",
            "agent_identity": "codex",
            "body_kind": body_kind,
        });
        if let Some(parent_event_id) = parent_event_id {
            raw.as_object_mut().unwrap().insert(
                "parent_event_id".to_string(),
                Value::String(parent_event_id.to_string()),
            );
        }
        if let Some(step_id) = step_id {
            raw.as_object_mut()
                .unwrap()
                .insert("step_id".to_string(), Value::String(step_id.to_string()));
        }
        RunAuditEvent {
            raw,
            event_id: event_id.to_string(),
            parent_event_id: parent_event_id.map(str::to_string),
            event_type: Some(event_type.to_string()),
            body_kind: Some(body_kind),
            timestamp: None,
            step_id: step_id.map(str::to_string),
        }
    }
}
