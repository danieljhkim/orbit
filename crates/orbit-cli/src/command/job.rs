use std::path::PathBuf;

use clap::{Args, Subcommand};
use orbit_common::types::{ActivityV2Spec, JobKind, JobV2Step, JobV2StepBody};
use orbit_core::command::job::{JobCatalogEntry, JobCatalogFilter};
use orbit_core::{JobRun, OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "Define, list, and manage job workflows")]
pub struct JobCommand {
    #[command(subcommand)]
    pub command: JobSubcommand,
}

impl Execute for JobCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum JobSubcommand {
    /// List all registered jobs
    List(JobListArgs),
    /// Show details of a specific job
    Show(JobShowArgs),
    /// Execute a schemaVersion 2 job by ID or YAML path
    Run(JobRunArgs),
    /// Internal worker entrypoint for persisted pipeline runs
    #[command(name = "run-pipeline-worker", hide = true)]
    RunPipelineWorker(JobRunPipelineWorkerArgs),
}

impl Execute for JobSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            JobSubcommand::List(args) => args.execute(runtime),
            JobSubcommand::Show(args) => args.execute(runtime),
            JobSubcommand::Run(args) => args.execute(runtime),
            JobSubcommand::RunPipelineWorker(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
#[command(
    after_help = "Examples:\n  orbit job list\n  orbit job list --all\n  orbit job list --kind subroutine\n  orbit job list --json"
)]
pub struct JobListArgs {
    /// Include disabled jobs
    #[arg(long)]
    pub all: bool,
    /// Filter to one v2 job kind.
    #[arg(long, value_enum)]
    pub kind: Option<JobKind>,
    /// Output full job objects as JSON
    #[arg(long)]
    pub json: bool,
    /// Output signal-tier JSON (job_id, target_id, state only)
    #[arg(long)]
    pub ops: bool,
}

impl Execute for JobListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let filter = job_catalog_filter(self.all, self.kind);
        if self.ops {
            let jobs = runtime.list_job_catalog_with_last_run(self.all, filter)?;
            let values = jobs
                .iter()
                .map(|(job, _)| job_catalog_to_signal_json(job))
                .collect::<Vec<_>>();
            return crate::output::json::print_pretty(&Value::Array(values));
        }

        let jobs_with_runs = runtime.list_job_catalog_with_last_run(self.all, filter)?;
        if self.json {
            let values = jobs_with_runs
                .iter()
                .map(|(job, last_run)| job_catalog_to_json_with_last_run(job, last_run.as_ref()))
                .collect::<Vec<_>>();
            crate::output::json::print_pretty(&Value::Array(values))
        } else {
            let mut table = crate::output::table::build_table(&[
                "JOB_ID",
                "KIND",
                "TARGET_TYPE",
                "TARGET_ID",
                "STATE",
                "LAST_RUN",
            ]);
            for (job, last_run) in &jobs_with_runs {
                use comfy_table::Cell;
                let (target_type, target_id) = job_catalog_target_summary(job);
                table.add_row(vec![
                    Cell::new(&job.job_id),
                    Cell::new(job.kind().to_string()),
                    Cell::new(target_type),
                    Cell::new(target_id),
                    crate::output::color::job_state_color_cell(&job.state().to_string()),
                    Cell::new(format_last_run(last_run.as_ref())),
                ]);
            }
            println!("{table}");
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct JobShowArgs {
    pub job_id: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for JobShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let job = runtime.show_job_catalog_entry(&self.job_id)?;
        if self.json {
            crate::output::json::print_pretty(&job_catalog_to_json_with_last_run(&job, None))
        } else {
            use crate::output::color::{bold, job_state_color};
            println!("{} {}", bold("Job ID:"), job.job_id.as_str());
            println!("{} {}", bold("Kind:"), job.kind());
            println!(
                "{} {}",
                bold("State:"),
                job_state_color(&job.state().to_string())
            );
            println!("{} {}", bold("Max Active Runs:"), job.max_active_runs());
            println!("{} {}", bold("Path:"), job.path.display());
            if let Some(default_input) = job.default_input() {
                let rendered = serde_json::to_string(default_input)
                    .unwrap_or_else(|_| "<invalid-json>".to_string());
                println!("{} {}", bold("Default Input:"), rendered);
            }
            println!("{} {}", bold("Steps:"), job.spec.steps.len());
            for (i, step) in job.spec.steps.iter().enumerate() {
                println!("  {}:", bold(&format!("Step {}", i + 1)));
                print_v2_step(step, 4);
            }
            Ok(())
        }
    }
}

#[derive(Args)]
#[command(
    after_help = "Examples:\n  orbit run job task_auto_pipeline\n  orbit run job task_auto_pipeline --input mode=local\n  orbit run job crates/orbit-core/assets/jobs/task_pipeline.yaml --input task_id=T123\n"
)]
pub struct JobRunArgs {
    /// Job ID from the catalog, or a direct path to a schemaVersion 2 job YAML.
    pub job_id: String,
    /// Input key=value pairs passed to all job steps (repeatable).
    /// Example: --input task_id=T123 --input base=main
    #[arg(long)]
    pub input: Vec<String>,
    /// Explicit execution backend override for `agent_loop` steps (§3.1).
    /// Precedence: this flag > `ORBIT_BACKEND` > `[runtime] backend` > `http`.
    /// Accepted values: `http`, `cli`, `auto`.
    #[arg(long)]
    pub backend: Option<String>,
    #[arg(long)]
    pub json: bool,
    /// Stream agent stderr to the terminal and tee stdout live for debugging.
    #[arg(long)]
    pub debug: bool,
}

impl Execute for JobRunArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let input = build_job_run_input(&self.input)?;
        let backend_flag =
            orbit_core::command::backend_resolver::parse_backend_flag(self.backend.as_deref())
                .map_err(OrbitError::InvalidInput)?;
        let direct_path = PathBuf::from(&self.job_id);
        if direct_path.exists() {
            if self.debug {
                return Err(OrbitError::InvalidInput(
                    "`orbit job run --debug` is not supported for schemaVersion 2 jobs; use the audit output instead.".to_string(),
                ));
            }
            let result = runtime.run_job_v2_from_yaml(&direct_path, input, backend_flag)?;
            let audit_jsonl_str = result
                .audit_jsonl
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "-".to_string());
            let backend_str = result.resolved_backend.as_str();
            if self.json {
                return crate::output::json::print_pretty(&json!({
                    "run_id": result.run_id,
                    "job_name": result.job_name,
                    "resolved_backend": backend_str,
                    "success": result.success,
                    "message": result.message,
                    "pipeline": result.pipeline,
                    "audit_jsonl": audit_jsonl_str,
                    "events_emitted": result.events_emitted,
                }));
            }
            println!(
                "run_id={};job={};backend={};success={};events={};audit_jsonl={}",
                result.run_id,
                result.job_name,
                backend_str,
                result.success,
                result.events_emitted,
                audit_jsonl_str,
            );
            if let Some(msg) = &result.message {
                println!("message: {msg}");
            }
            println!(
                "pipeline: {}",
                serde_json::to_string_pretty(&result.pipeline).unwrap_or_default()
            );
            return Ok(());
        }

        let job = runtime.show_job_catalog_entry(&self.job_id)?;
        if self.debug {
            return Err(OrbitError::InvalidInput(
                "`orbit job run --debug` is not supported for schemaVersion 2 jobs; use the audit output instead.".to_string(),
            ));
        }
        if job.kind() == JobKind::Subroutine {
            return Err(OrbitError::InvalidInput(build_subroutine_run_error(&job)));
        }
        let result = runtime.run_job_v2_from_yaml(&job.path, input, backend_flag)?;
        let audit_jsonl_str = result
            .audit_jsonl
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "-".to_string());
        let backend_str = result.resolved_backend.as_str();
        if self.json {
            crate::output::json::print_pretty(&json!({
                "run_id": result.run_id,
                "job_id": job.job_id.clone(),
                "kind": job.kind().to_string(),
                "resolved_backend": backend_str,
                "success": result.success,
                "message": result.message,
                "pipeline": result.pipeline,
                "audit_jsonl": audit_jsonl_str,
                "events_emitted": result.events_emitted,
            }))
        } else {
            println!(
                "run_id={};job_id={};kind={};backend={};success={};events={};audit_jsonl={}",
                result.run_id,
                job.job_id.as_str(),
                job.kind(),
                backend_str,
                result.success,
                result.events_emitted,
                audit_jsonl_str,
            );
            if let Some(msg) = &result.message {
                println!("message: {msg}");
            }
            println!(
                "pipeline: {}",
                serde_json::to_string_pretty(&result.pipeline).unwrap_or_default()
            );
            Ok(())
        }
    }
}

fn format_last_run(last_run: Option<&JobRun>) -> String {
    match last_run {
        None => "never".to_string(),
        Some(run) => {
            let ts = run
                .finished_at
                .or(run.started_at)
                .unwrap_or(run.scheduled_at);
            format!("{} {}", run.state, ts.format("%Y-%m-%dT%H:%M:%SZ"))
        }
    }
}

fn job_catalog_filter(include_disabled: bool, kind: Option<JobKind>) -> JobCatalogFilter {
    match kind {
        Some(kind) => JobCatalogFilter::Kind(kind),
        None if include_disabled => JobCatalogFilter::All,
        None => JobCatalogFilter::WorkflowsOnly,
    }
}

fn job_catalog_target_summary(job: &JobCatalogEntry) -> (String, String) {
    job.spec
        .steps
        .first()
        .map(v2_step_target_summary)
        .unwrap_or_else(|| ("-".to_string(), "-".to_string()))
}

pub(crate) fn job_catalog_to_json_with_last_run(
    job: &JobCatalogEntry,
    last_run: Option<&JobRun>,
) -> Value {
    let mut value = json!({
        "job_id": job.job_id.clone(),
        "kind": job.kind().to_string(),
        "state": job.state().to_string(),
        "default_input": job.spec.default_input,
        "max_active_runs": job.spec.max_active_runs,
        "steps": job.spec.steps.iter().map(job_v2_step_to_json).collect::<Vec<_>>(),
        "path": job.path.display().to_string(),
    });
    value["last_run_state"] = last_run
        .map(|r| serde_json::Value::String(r.state.to_string()))
        .unwrap_or(serde_json::Value::Null);
    value["last_run_at"] = last_run
        .and_then(|r| r.finished_at.or(r.started_at).or(Some(r.scheduled_at)))
        .map(|ts| serde_json::Value::String(ts.to_rfc3339()))
        .unwrap_or(serde_json::Value::Null);
    value
}

fn job_catalog_to_signal_json(job: &JobCatalogEntry) -> Value {
    let (_, target_id) = job_catalog_target_summary(job);
    json!({
        "job_id": job.job_id.clone(),
        "target_id": target_id,
        "state": job.state().to_string(),
    })
}

fn job_v2_step_to_json(step: &JobV2Step) -> Value {
    let mut value = json!({
        "id": step.id.clone(),
        "when": step.when,
        "retry": step.retry,
    });
    match &step.body {
        JobV2StepBody::TargetRef(target) => {
            value["body"] = json!({
                "kind": "target_ref",
                "target": target.target.clone(),
                "default_input": target.default_input,
                "timeout_seconds": target.timeout_seconds,
                "session": target.session,
            });
        }
        JobV2StepBody::Target(target) => {
            value["body"] = json!({
                "kind": "target",
                "default_input": target.default_input,
                "timeout_seconds": target.timeout_seconds,
                "session": target.session,
                "spec": target.spec,
            });
        }
        JobV2StepBody::Parallel { parallel } => {
            value["body"] = json!({
                "kind": "parallel",
                "join": parallel.join,
                "branches": parallel.branches.iter().map(job_v2_step_to_json).collect::<Vec<_>>(),
            });
        }
        JobV2StepBody::FanOut { fan_out, fan_in } => {
            value["body"] = json!({
                "kind": "fan_out",
                "items": fan_out.items,
                "max_workers": fan_out.max_workers,
                "worker": job_v2_step_to_json(&fan_out.worker),
                "fan_in": fan_in,
            });
        }
        JobV2StepBody::Loop { loop_ } => {
            value["body"] = json!({
                "kind": "loop",
                "max_iterations": loop_.max_iterations,
                "break_when": loop_.break_when,
                "steps": loop_.steps.iter().map(job_v2_step_to_json).collect::<Vec<_>>(),
            });
        }
    }
    value
}

pub(crate) fn job_run_to_json(run: &JobRun) -> Value {
    let last = run.steps.last();
    json!({
        "run_id": run.run_id,
        "job_id": run.job_id,
        "attempt": run.attempt,
        "state": run.state.to_string(),
        "scheduled_at": run.scheduled_at.to_rfc3339(),
        "started_at": run.started_at.map(|v| v.to_rfc3339()),
        "finished_at": run.finished_at.map(|v| v.to_rfc3339()),
        "duration_ms": run.duration_ms,
        "exit_code": last.and_then(|s| s.exit_code),
        "agent_response_json": last.and_then(|s| s.agent_response_json.as_ref()),
        "error_code": last.and_then(|s| s.error_code.as_deref()),
        "error_message": last.and_then(|s| s.error_message.as_deref()),
        "knowledge_metrics": run.knowledge_metrics,
        "steps": run.steps.iter().map(|s| json!({
            "step_index": s.step_index,
            "target_type": s.target_type.to_string(),
            "target_id": s.target_id,
            "state": s.state.to_string(),
            "started_at": s.started_at.map(|v| v.to_rfc3339()),
            "finished_at": s.finished_at.map(|v| v.to_rfc3339()),
            "duration_ms": s.duration_ms,
            "exit_code": s.exit_code,
            "agent_response_json": s.agent_response_json,
            "error_code": s.error_code,
            "error_message": s.error_message,
        })).collect::<Vec<_>>(),
        "created_at": run.created_at.to_rfc3339(),
    })
}

pub(crate) fn summarize_error_message(raw: Option<&str>) -> String {
    let value = raw.unwrap_or("-").replace('\n', " ");
    if value.chars().count() <= 120 {
        return value;
    }
    let truncated = value.chars().take(120).collect::<String>();
    format!("{truncated}...")
}

fn build_subroutine_run_error(job: &JobCatalogEntry) -> String {
    format!(
        "job '{}' declares `kind: subroutine` and cannot be run directly (asset: {}).",
        job.job_id.as_str(),
        job.path.display()
    )
}

fn print_v2_step(step: &JobV2Step, indent: usize) {
    use crate::output::color::bold;

    let pad = " ".repeat(indent);
    println!("{pad}{} {}", bold("ID:"), step.id.as_str());
    if let Some(when) = &step.when {
        println!("{pad}{} {}", bold("When:"), when);
    }
    if let Some(retry) = &step.retry {
        println!("{pad}{} {:?}", bold("Retry:"), retry);
    }
    match &step.body {
        JobV2StepBody::TargetRef(target) => {
            println!("{pad}{} {}", bold("Target Ref:"), target.target.as_str());
            if let Some(session) = &target.session {
                println!("{pad}{} {}", bold("Session:"), session);
            }
            println!("{pad}{} {}", bold("Timeout (s):"), target.timeout_seconds);
        }
        JobV2StepBody::Target(target) => {
            match &target.spec {
                ActivityV2Spec::AgentLoop(spec) => {
                    println!("{pad}{} agent_loop", bold("Activity Type:"));
                    println!("{pad}{} {}", bold("Provider:"), spec.provider.as_str());
                    println!("{pad}{} {}", bold("Backend:"), spec.backend.as_str());
                    if let Some(model) = &spec.model {
                        println!("{pad}{} {}", bold("Model:"), model);
                    }
                }
                ActivityV2Spec::Groundhog(spec) => {
                    println!("{pad}{} groundhog", bold("Activity Type:"));
                    println!("{pad}{} {}", bold("Provider:"), spec.provider.as_str());
                    println!("{pad}{} http", bold("Backend:"));
                    println!(
                        "{pad}{} {}",
                        bold("Attempt Budget Default:"),
                        spec.attempt_budget_default
                    );
                    if let Some(model) = &spec.model {
                        println!("{pad}{} {}", bold("Model:"), model);
                    }
                }
                ActivityV2Spec::Deterministic(spec) => {
                    println!("{pad}{} deterministic", bold("Activity Type:"));
                    println!("{pad}{} {}", bold("Action:"), spec.action.as_str());
                }
                ActivityV2Spec::Shell(spec) => {
                    println!("{pad}{} shell", bold("Activity Type:"));
                    println!("{pad}{} {}", bold("Program:"), spec.program.as_str());
                }
            }
            if let Some(session) = &target.session {
                println!("{pad}{} {}", bold("Session:"), session);
            }
            println!("{pad}{} {}", bold("Timeout (s):"), target.timeout_seconds);
        }
        JobV2StepBody::Parallel { parallel } => {
            println!("{pad}{} parallel", bold("Body:"));
            println!("{pad}{} {:?}", bold("Join:"), parallel.join);
            println!("{pad}{} {}", bold("Branches:"), parallel.branches.len());
            for branch in &parallel.branches {
                print_v2_step(branch, indent + 2);
            }
        }
        JobV2StepBody::FanOut { fan_out, fan_in } => {
            println!("{pad}{} fan_out", bold("Body:"));
            println!("{pad}{} {}", bold("Items:"), fan_out.items.as_str());
            println!("{pad}{} {}", bold("Max Workers:"), fan_out.max_workers);
            println!("{pad}{} {:?}", bold("Fan In:"), fan_in);
            print_v2_step(&fan_out.worker, indent + 2);
        }
        JobV2StepBody::Loop { loop_ } => {
            println!("{pad}{} loop", bold("Body:"));
            println!("{pad}{} {}", bold("Max Iterations:"), loop_.max_iterations);
            if let Some(break_when) = &loop_.break_when {
                println!("{pad}{} {}", bold("Break When:"), break_when);
            }
            for nested in &loop_.steps {
                print_v2_step(nested, indent + 2);
            }
        }
    }
}

fn v2_step_target_summary(step: &JobV2Step) -> (String, String) {
    match &step.body {
        JobV2StepBody::TargetRef(target) => ("activity_ref".to_string(), target.target.clone()),
        JobV2StepBody::Target(target) => match &target.spec {
            ActivityV2Spec::AgentLoop(spec) => (
                "agent_loop".to_string(),
                spec.model
                    .clone()
                    .unwrap_or_else(|| spec.provider.as_str().to_string()),
            ),
            ActivityV2Spec::Groundhog(spec) => (
                "groundhog".to_string(),
                spec.model
                    .clone()
                    .unwrap_or_else(|| spec.provider.as_str().to_string()),
            ),
            ActivityV2Spec::Deterministic(spec) => {
                ("deterministic".to_string(), spec.action.clone())
            }
            ActivityV2Spec::Shell(spec) => ("shell".to_string(), spec.program.clone()),
        },
        JobV2StepBody::Parallel { .. } => ("parallel".to_string(), step.id.clone()),
        JobV2StepBody::FanOut { .. } => ("fan_out".to_string(), step.id.clone()),
        JobV2StepBody::Loop { .. } => ("loop".to_string(), step.id.clone()),
    }
}

#[derive(Args)]
pub struct JobRunPipelineWorkerArgs {
    /// Persisted run ID to claim and execute.
    pub run_id: String,
}

impl Execute for JobRunPipelineWorkerArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.execute_pipeline_run_worker(&self.run_id)
    }
}

fn build_job_run_input(pairs: &[String]) -> Result<Value, OrbitError> {
    let mut map = serde_json::Map::new();
    for pair in pairs {
        let (key, value) = pair.split_once('=').ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "invalid --input entry \"{pair}\": expected key=value"
            ))
        })?;
        let key = key.trim();
        if key.is_empty() {
            return Err(OrbitError::InvalidInput(format!(
                "invalid --input entry \"{pair}\": key must not be empty"
            )));
        }
        map.insert(key.to_string(), Value::String(value.to_string()));
    }
    Ok(Value::Object(map))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::command::Cli;

    #[test]
    fn rejects_removed_job_run_inspection_aliases() {
        assert!(Cli::try_parse_from(["orbit", "job", "history", "task_auto_pipeline"]).is_err());
        assert!(Cli::try_parse_from(["orbit", "job", "run-state", "jrun-1"]).is_err());
    }
}
