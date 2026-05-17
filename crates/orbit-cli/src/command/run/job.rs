use std::path::PathBuf;

use clap::Args;
use orbit_common::types::JobKind;
use orbit_common::types::PipelineState;
use orbit_core::command::job::JobCatalogEntry;
use orbit_core::{JobRun, OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

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
    /// Precedence: this flag > `ORBIT_BACKEND` > `[runtime] backend` > `cli`.
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

#[derive(Args)]
#[command(after_help = "Examples:\n  orbit job replay jrun-task_auto_pipeline-20260505T061300.000")]
pub struct JobReplayArgs {
    /// Source job run ID to replay from step 0.
    pub run_id: String,
    /// Output replay result as JSON.
    #[arg(long)]
    pub json: bool,
}

impl Execute for JobReplayArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let source_run_id = self.run_id;
        let result = runtime.replay_job_run(&source_run_id)?;
        let audit_jsonl_str = result
            .audit_jsonl
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "-".to_string());
        let backend_str = result.resolved_backend.as_str();
        if self.json {
            return crate::output::json::print_pretty(&json!({
                "run_id": result.run_id,
                "source_run_id": source_run_id,
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
            "run_id={};replayed_from={};job={};backend={};success={};events={};audit_jsonl={}",
            result.run_id,
            source_run_id,
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
        Ok(())
    }
}

pub(crate) fn job_run_to_json(run: &JobRun) -> Value {
    job_run_to_json_with_state(run, None)
}

pub(crate) fn job_run_to_json_with_state(run: &JobRun, state: Option<&PipelineState>) -> Value {
    let last = run.steps.last();
    let state = (!run.state.is_terminal()).then_some(state).flatten();
    let waiting_on_deps = state
        .and_then(|state| state.waiting_on_deps.as_ref())
        .filter(|values| !values.is_empty());
    let waiting_on_locks = state
        .and_then(|state| state.waiting_on_locks.as_ref())
        .filter(|values| !values.is_empty());
    json!({
        "run_id": run.run_id,
        "job_id": run.job_id,
        "attempt": run.attempt,
        "state": run.state.to_string(),
        "waiting_on_deps": waiting_on_deps,
        "waiting_on_locks": waiting_on_locks,
        "scheduled_at": run.scheduled_at.to_rfc3339(),
        "started_at": run.started_at.map(|v| v.to_rfc3339()),
        "finished_at": run.finished_at.map(|v| v.to_rfc3339()),
        "duration_ms": run.duration_ms,
        "retry_source_run_id": run.retry_source_run_id,
        "exit_code": last.and_then(|s| s.exit_code),
        "agent_response_json": last.and_then(|s| s.agent_response_json.as_ref()),
        "error_code": last.and_then(|s| s.error_code.as_deref()),
        "error_message": last.and_then(|s| s.error_message.as_deref()),
        "knowledge_metrics": run.knowledge_metrics,
        "resolved_crew": run.resolved_crew,
        "planner_model": run.planner_model,
        "implementer_model": run.implementer_model,
        "reviewer_model": run.reviewer_model,
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

fn build_subroutine_run_error(job: &JobCatalogEntry) -> String {
    format!(
        "job '{}' declares `kind: subroutine` and cannot be run directly (asset: {}).",
        job.job_id.as_str(),
        job.path.display()
    )
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
    use super::*;
    use chrono::Utc;
    use orbit_common::types::JobRunState;
    use orbit_core::NotFoundKind;

    fn test_run(state: JobRunState) -> JobRun {
        let now = Utc::now();
        JobRun {
            run_id: "jrun-test".to_string(),
            job_id: "task_gate_pipeline".to_string(),
            attempt: 1,
            state,
            scheduled_at: now,
            started_at: Some(now),
            finished_at: None,
            duration_ms: None,
            created_at: now,
            pid: None,
            pid_start_time: None,
            input: None,
            retry_source_run_id: None,
            knowledge_metrics: None,
            resolved_crew: None,
            planner_model: None,
            implementer_model: None,
            reviewer_model: None,
            steps: Vec::new(),
        }
    }

    fn write_replay_job(runtime: &OrbitRuntime, name: &str) -> PathBuf {
        let jobs_dir = runtime.data_root().join("resources/jobs");
        std::fs::create_dir_all(&jobs_dir).expect("create jobs dir");
        let path = jobs_dir.join(format!("{name}.yaml"));
        std::fs::write(
            &path,
            format!(
                r#"schemaVersion: 2
kind: Job
metadata:
  name: {name}
spec:
  state: enabled
  kind: workflow
  steps:
    - id: nap
      spec:
        type: deterministic
        action: sleep
        config: {{}}
"#
            ),
        )
        .expect("write replay job");
        path
    }

    #[test]
    fn job_run_json_includes_waiting_reasons_from_state() {
        let run = test_run(JobRunState::Running);
        let mut state = PipelineState::new(run.run_id.clone(), run.job_id.clone(), json!({}));
        state.set_waiting_reasons(
            Some(vec!["ORB-1".to_string()]),
            Some(vec!["file:src/lib.rs".to_string()]),
        );

        let value = job_run_to_json_with_state(&run, Some(&state));

        assert_eq!(value["waiting_on_deps"], json!(["ORB-1"]));
        assert_eq!(value["waiting_on_locks"], json!(["file:src/lib.rs"]));
    }

    #[test]
    fn job_run_json_omits_stale_waiting_reasons_for_terminal_run() {
        let run = test_run(JobRunState::Success);
        let mut state = PipelineState::new(run.run_id.clone(), run.job_id.clone(), json!({}));
        state.set_waiting_reasons(
            Some(vec!["ORB-1".to_string()]),
            Some(vec!["file:src/lib.rs".to_string()]),
        );

        let value = job_run_to_json_with_state(&run, Some(&state));

        assert_eq!(value["waiting_on_deps"], Value::Null);
        assert_eq!(value["waiting_on_locks"], Value::Null);
    }

    #[test]
    fn job_replay_args_execute_creates_linked_run() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let job_path = write_replay_job(&runtime, "cli_replay_success");
        let source = runtime
            .run_job_v2_from_yaml(&job_path, json!({ "seconds": 0 }), None)
            .expect("source run");

        JobReplayArgs {
            run_id: source.run_id.clone(),
            json: true,
        }
        .execute(&runtime)
        .expect("replay run");

        let history = runtime
            .job_history("cli_replay_success")
            .expect("job history");
        assert!(history.iter().any(|run| {
            run.retry_source_run_id.as_deref() == Some(source.run_id.as_str())
                && run.state == orbit_common::types::JobRunState::Success
        }));
    }

    #[test]
    fn job_replay_args_execute_unknown_run_returns_error() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let error = JobReplayArgs {
            run_id: "jrun-missing".to_string(),
            json: true,
        }
        .execute(&runtime)
        .expect_err("unknown source run should fail");

        assert!(matches!(
            error,
            OrbitError::NotFound {
                kind: NotFoundKind::JobRun,
                ..
            }
        ));
    }
}
