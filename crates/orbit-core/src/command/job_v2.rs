//! `orbit job run <yaml-path>` — schemaVersion 2 job entrypoint.
//!
//! Mirrors `activity_v2::run_activity_v2_from_yaml`: reads the YAML, routes
//! through the two-pass loader, and dispatches via the Phase 3 DAG executor.
//! orbit-core never names orbit-agent types — transport/session construction
//! lives below the boundary in `orbit_engine::activity_job::job_executor`.

use std::path::{Path, PathBuf};

use orbit_common::types::activity_job::{
    Backend, V2AuditEventKind, load_job_asset, resolve_job_backends,
    validate_job_loop_session_backends,
};
use orbit_common::types::{
    JobRun, JobRunState, JobTargetType, OrbitError, OrbitEvent, PipelineState,
};
use orbit_engine::activity_job::{
    DispatchError, JobOutcome, V2AuditWriter, execute_job, resolve_job_catalog_refs_for_execution,
};
use orbit_store::JobRunStepParams;
use serde_json::Value;

use crate::OrbitRuntime;
use crate::command::SYSTEM_AUDIT_IDENTITY;

#[derive(Debug, Clone)]
pub struct V2JobRunResult {
    pub run_id: String,
    pub job_name: String,
    pub success: bool,
    pub pipeline: Value,
    pub message: Option<String>,
    pub audit_jsonl: Option<PathBuf>,
    pub events_emitted: usize,
    /// Resolved backend applied at load time to every `agent_loop` step in
    /// the DAG. Recorded so smokes can inspect the precedence outcome.
    pub resolved_backend: Backend,
}

impl OrbitRuntime {
    /// Execute a v2 Job from a YAML file. Returns a structural result and the
    /// path to the persisted §7 envelope JSONL. The file must declare
    /// `schemaVersion: 2` and `kind: Job`; v1 files are rejected.
    pub fn run_job_v2_from_yaml(
        &self,
        yaml_path: &Path,
        input: Value,
        backend_flag: Option<Backend>,
    ) -> Result<V2JobRunResult, OrbitError> {
        let job_name = load_job_name(yaml_path)?;
        let scheduled_at = chrono::Utc::now();
        let run = self.stores().jobs().insert_run(
            &job_name,
            1,
            scheduled_at,
            Some(input.clone()),
            None,
        )?;
        let initial_state =
            PipelineState::new(run.run_id.clone(), run.job_id.clone(), input.clone());
        self.stores()
            .jobs()
            .write_run_state(&run.run_id, &initial_state)?;

        let started_at = chrono::Utc::now();
        let changed =
            self.stores()
                .jobs()
                .mark_run_running(&run.run_id, started_at, std::process::id())?;
        if !changed {
            return Err(OrbitError::JobRunNotFound(run.run_id));
        }
        self.record_event(OrbitEvent::JobRunStarted {
            job_id: run.job_id.clone(),
            run_id: run.run_id.clone(),
            attempt: run.attempt,
        })?;

        let outcome = self.run_job_v2_from_yaml_with_run_id(
            yaml_path,
            input.clone(),
            backend_flag,
            Some(run.run_id.clone()),
        );
        let finished_at = chrono::Utc::now();
        let duration_ms = Some(
            finished_at
                .signed_duration_since(started_at)
                .num_milliseconds()
                .max(0) as u64,
        );

        match outcome {
            Ok(result) => {
                let final_state = if result.success {
                    JobRunState::Success
                } else {
                    JobRunState::Failed
                };
                self.persist_direct_v2_run_state(&run, &input, &result, final_state)?;
                if result.success {
                    self.record_direct_v2_success_step(&run, started_at, finished_at, &result)?;
                } else {
                    let fallback = "job completed with success=false but emitted no failure detail";
                    let message = result.message.as_deref().unwrap_or(fallback);
                    let _ =
                        self.record_pipeline_failure_step(&run, started_at, finished_at, message);
                }
                self.stores().jobs().finalize_run(
                    &run.run_id,
                    final_state,
                    finished_at,
                    duration_ms,
                )?;
                self.record_event(OrbitEvent::JobRunCompleted {
                    job_id: run.job_id.clone(),
                    run_id: run.run_id.clone(),
                    state: final_state.to_string(),
                })?;
                Ok(result)
            }
            Err(error) => {
                let _ = self.record_pipeline_failure_step(
                    &run,
                    started_at,
                    finished_at,
                    &error.to_string(),
                );
                self.stores().jobs().finalize_run(
                    &run.run_id,
                    JobRunState::Failed,
                    finished_at,
                    duration_ms,
                )?;
                self.record_event(OrbitEvent::JobRunCompleted {
                    job_id: run.job_id.clone(),
                    run_id: run.run_id.clone(),
                    state: JobRunState::Failed.to_string(),
                })?;
                Err(error)
            }
        }
    }

    pub fn run_job_v2_from_yaml_with_run_id(
        &self,
        yaml_path: &Path,
        input: Value,
        backend_flag: Option<Backend>,
        run_id_override: Option<String>,
    ) -> Result<V2JobRunResult, OrbitError> {
        let yaml = std::fs::read_to_string(yaml_path).map_err(|err| {
            OrbitError::InvalidInput(format!("read {}: {err}", yaml_path.display()))
        })?;
        let mut asset = load_job_asset(&yaml).map_err(|err| {
            OrbitError::InvalidInput(format!("load {}: {err}", yaml_path.display()))
        })?;

        // Phase 4: resolve `target: activity:<name>` refs before any other
        // pass, so backend-resolution + loader-rejection see concrete specs.
        let catalog = self
            .v2_activity_catalog()
            .map_err(|err| OrbitError::InvalidInput(format!("build activity catalog: {err}")))?;
        resolve_job_catalog_refs_for_execution(&mut asset.spec, &catalog).map_err(
            |err| match err {
                DispatchError::JobValidation(message) => OrbitError::JobValidation(message),
                other => OrbitError::InvalidInput(format!("{other}")),
            },
        )?;

        // §3.1 resolution: replace every `Auto` with a concrete backend.
        let resolution = self.resolve_v2_backend(backend_flag);
        resolve_job_backends(&mut asset.spec, resolution.backend);

        // §3.2 loader rejection: any `loop:`-nested step with `session:`
        // binding must resolve to `backend: http`. We reject at load time so
        // CLI-mode runs never start a DAG they can't finish.
        validate_job_loop_session_backends(&asset.spec, &yaml_path.display().to_string())
            .map_err(|err| OrbitError::InvalidInput(format!("{err}")))?;
        let run_id = run_id_override.unwrap_or_else(|| {
            format!(
                "job-{}-{}",
                asset.name,
                chrono::Utc::now().format("%Y%m%dT%H%M%S%.3f")
            )
        });

        let audit_root = self.paths().audit_dir.clone();
        let workspace_path = self.paths().repo_root.clone();
        let writer = V2AuditWriter::with_disk_sinks(
            &audit_root,
            &run_id,
            SYSTEM_AUDIT_IDENTITY,
            Some(workspace_path.as_path()),
        )
        .map_err(|err| OrbitError::Execution(format!("audit sinks: {err}")))?;
        let audit_jsonl = writer.envelope_log_path();

        self.record_event(OrbitEvent::ActivityRunStarted {
            id: asset.name.clone(),
        })?;
        let _ = writer.emit(V2AuditEventKind::RunStarted {
            job_name: format!("cli:{}", asset.name),
        });

        let outcome_res: Result<JobOutcome, OrbitError> =
            execute_job(&asset.spec, input, &run_id, writer.clone(), self)
                .map_err(|err| OrbitError::Execution(format!("v2 job dispatch: {err}")));

        let outcome_str = match &outcome_res {
            Ok(o) if o.success => "success",
            Ok(_) => "failed",
            Err(_) => "error",
        };
        let _ = writer.emit(V2AuditEventKind::RunFinished {
            outcome: outcome_str.to_string(),
        });
        self.record_event(OrbitEvent::ActivityRunCompleted {
            id: asset.name.clone(),
            state: outcome_str.to_string(),
        })?;

        let events_count = writer
            .events_snapshot()
            .map(|s| s.len())
            .unwrap_or_default();

        match outcome_res {
            Ok(o) => Ok(V2JobRunResult {
                run_id,
                job_name: asset.name,
                success: o.success,
                pipeline: o.pipeline,
                message: o.message,
                audit_jsonl,
                events_emitted: events_count,
                resolved_backend: resolution.backend,
            }),
            Err(err) => Err(err),
        }
    }

    fn persist_direct_v2_run_state(
        &self,
        run: &JobRun,
        input: &Value,
        result: &V2JobRunResult,
        final_state: JobRunState,
    ) -> Result<(), OrbitError> {
        let mut state = self.read_run_state(&run.run_id)?.unwrap_or_else(|| {
            PipelineState::new(run.run_id.clone(), run.job_id.clone(), input.clone())
        });
        state.sync_pipeline(result.pipeline.clone());
        state.record_step(0, final_state, Some(result.pipeline.clone()), None);
        self.stores().jobs().write_run_state(&run.run_id, &state)
    }

    fn record_direct_v2_success_step(
        &self,
        run: &JobRun,
        started_at: chrono::DateTime<chrono::Utc>,
        finished_at: chrono::DateTime<chrono::Utc>,
        result: &V2JobRunResult,
    ) -> Result<(), OrbitError> {
        let duration_ms = Some(
            finished_at
                .signed_duration_since(started_at)
                .num_milliseconds()
                .max(0) as u64,
        );
        self.stores().jobs().complete_run_step(
            &run.run_id,
            &JobRunStepParams {
                step_index: 0,
                target_type: JobTargetType::Job,
                target_id: run.job_id.clone(),
                started_at,
                finished_at,
                duration_ms,
                exit_code: Some(0),
                agent_response_json: Some(result.pipeline.clone()),
                state: JobRunState::Success,
                error_code: None,
                error_message: None,
            },
        )?;
        Ok(())
    }
}

fn load_job_name(yaml_path: &Path) -> Result<String, OrbitError> {
    let yaml = std::fs::read_to_string(yaml_path)
        .map_err(|err| OrbitError::InvalidInput(format!("read {}: {err}", yaml_path.display())))?;
    let asset = load_job_asset(&yaml)
        .map_err(|err| OrbitError::InvalidInput(format!("load {}: {err}", yaml_path.display())))?;
    Ok(asset.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use chrono::Utc;
    use orbit_common::types::{ExecutorDef, ExecutorType};
    use orbit_store::InvocationQuery;
    use serde_json::json;
    use tempfile::tempdir;

    fn test_runtime() -> (tempfile::TempDir, OrbitRuntime, PathBuf, PathBuf) {
        let root = tempdir().expect("create tempdir");
        let global_root = root.path().join("global");
        let repo_root = root.path().join("repo");
        let workspace_root = repo_root.join(".orbit");
        std::fs::create_dir_all(&global_root).expect("create global root");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        let runtime =
            OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
        (root, runtime, repo_root, global_root)
    }

    fn write_job(path: &Path, name: &str, action: &str) {
        let yaml = format!(
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
        action: {action}
        config: {{}}
"#
        );
        std::fs::write(path, yaml).expect("write job yaml");
    }

    fn write_cli_metrics_job(path: &Path, name: &str) {
        let yaml = format!(
            r#"schemaVersion: 2
kind: Job
metadata:
  name: {name}
spec:
  state: enabled
  kind: workflow
  steps:
    - id: codex_metrics
      spec:
        type: agent_loop
        instruction: "emit a successful Orbit envelope"
        tools: [fs.read]
        on_denial: terminate
        max_iterations: 1
        model: gpt-test
        backend: cli
        provider: codex
        wall_clock_timeout_seconds: 30
"#
        );
        std::fs::write(path, yaml).expect("write cli metrics job yaml");
    }

    #[cfg(unix)]
    fn write_fake_codex(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        std::fs::write(
            path,
            r#"#!/bin/sh
cat >/dev/null
printf '%s\n' '{"type":"thread.started","thread_id":"fake"}'
printf '%s\n' '{"type":"item.started","item":{"id":"item_1","type":"command_execution","command":"orbit metrics","aggregated_output":"","exit_code":null,"status":"in_progress"}}'
printf '%s\n' '{"type":"item.completed","item":{"id":"item_1","type":"command_execution","command":"orbit metrics","aggregated_output":"ok","exit_code":0,"status":"completed"}}'
printf '%s\n' '{"schemaVersion":1,"status":"success","result":{"ok":true},"error":null}'
printf '%s\n' '{"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":25,"output_tokens":12}}'
"#,
        )
        .expect("write fake codex");
        let mut permissions = std::fs::metadata(path)
            .expect("fake codex metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("chmod fake codex");
    }

    #[test]
    fn direct_yaml_run_persists_history_and_run_state() {
        let (_root, runtime, repo_root, _global_root) = test_runtime();
        let yaml_path = repo_root.join("qa_sleep.yaml");
        write_job(&yaml_path, "qa_sleep", "sleep");

        let result = runtime
            .run_job_v2_from_yaml(&yaml_path, json!({ "seconds": 0 }), None)
            .expect("direct job run succeeds");

        let run = runtime.show_job_run(&result.run_id).expect("stored run");
        assert_eq!(run.job_id, "qa_sleep");
        assert_eq!(run.state, JobRunState::Success);
        assert_eq!(run.steps.len(), 1);

        let history = runtime.job_history("qa_sleep").expect("job history");
        assert!(history.iter().any(|run| run.run_id == result.run_id));

        let state = runtime
            .read_run_state(&result.run_id)
            .expect("read run state")
            .expect("persisted run state");
        assert_eq!(state.run_id, result.run_id);
        assert!(state.pipeline.get("nap").is_some());
        assert!(state.step_outputs.contains_key(&0));

        let audit_jsonl = result.audit_jsonl.as_ref().expect("audit jsonl path");
        let expected_audit_jsonl = repo_root
            .join(".orbit/state/audit/v2_loop")
            .join(format!("{}.jsonl", result.run_id));
        assert_eq!(audit_jsonl, &expected_audit_jsonl);
        assert!(expected_audit_jsonl.exists());
        let first_line = std::fs::read_to_string(&expected_audit_jsonl)
            .expect("read audit jsonl")
            .lines()
            .next()
            .expect("audit jsonl has a first event")
            .to_string();
        let first_event: serde_json::Value =
            serde_json::from_str(&first_line).expect("parse first audit event");
        assert_eq!(
            first_event
                .get("agent_identity")
                .and_then(serde_json::Value::as_str),
            Some(SYSTEM_AUDIT_IDENTITY)
        );
        assert!(
            repo_root
                .join(".orbit/state/audit/loop")
                .join(format!("{}.jsonl", result.run_id))
                .exists()
        );
        assert!(!repo_root.join(".orbit/audit").exists());
    }

    #[test]
    fn direct_catalog_run_is_visible_in_history() {
        let (_root, runtime, repo_root, global_root) = test_runtime();
        let jobs_dir = global_root.join("resources/jobs");
        std::fs::create_dir_all(&jobs_dir).expect("create jobs dir");
        let yaml_path = jobs_dir.join("qa_catalog_sleep.yaml");
        write_job(&yaml_path, "qa_catalog_sleep", "sleep");

        let catalog = runtime
            .show_job_catalog_entry("qa_catalog_sleep")
            .expect("catalog entry");
        let result = runtime
            .run_job_v2_from_yaml(&catalog.path, json!({ "seconds": 0 }), None)
            .expect("catalog job run succeeds");

        let history = runtime
            .job_history("qa_catalog_sleep")
            .expect("catalog history");
        assert!(history.iter().any(|run| run.run_id == result.run_id));
        assert!(repo_root.join(".orbit/state/job-runs").exists());
    }

    #[cfg(unix)]
    #[test]
    fn v2_cli_agent_loop_persists_invocation_metrics() {
        let (_root, runtime, repo_root, _global_root) = test_runtime();
        let fake_bin = repo_root.join("codex");
        write_fake_codex(&fake_bin);

        let now = Utc::now();
        runtime
            .upsert_executor_def(&ExecutorDef {
                name: "codex".to_string(),
                executor_type: ExecutorType::DirectAgent,
                command: Some(fake_bin.display().to_string()),
                args: Vec::new(),
                stdout_format: None,
                models: HashMap::new(),
                timeout_seconds: None,
                env: HashMap::new(),
                sandbox: None,
                allow_fallback: false,
                created_at: now,
                updated_at: now,
            })
            .expect("seed fake codex executor");

        let yaml_path = repo_root.join("qa_cli_metrics.yaml");
        write_cli_metrics_job(&yaml_path, "qa_cli_metrics");

        let result = runtime
            .run_job_v2_from_yaml(
                &yaml_path,
                json!({"prompt": "collect metrics", "task_id": "TTEST-1"}),
                None,
            )
            .expect("cli metrics job succeeds");

        let records = runtime
            .invocation_records(InvocationQuery {
                job_run_id: Some(result.run_id.clone()),
                limit: 10,
                ..InvocationQuery::default()
            })
            .expect("query invocation records");
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.activity_id, "codex_metrics");
        assert_eq!(record.agent, "codex");
        assert_eq!(record.model.as_deref(), Some("gpt-test"));
        assert_eq!(record.input_tokens, 100);
        assert_eq!(record.cache_read_tokens, 25);
        assert_eq!(record.output_tokens, 12);
        assert_eq!(record.task_ids, ["TTEST-1"]);
        assert_eq!(record.tool_call_count, 1);
        assert_eq!(record.tool_calls[0].tool_name, "command_execution");

        let activity = runtime
            .activity_invocation_metrics()
            .expect("activity metrics");
        assert!(activity.iter().any(|row| {
            row.activity_id == "codex_metrics"
                && row.agent == "codex"
                && row.model.as_deref() == Some("gpt-test")
                && row.total_input_tokens == 100
                && row.total_output_tokens == 12
                && row.total_tool_calls == 1
        }));

        let tools = runtime.tool_invocation_metrics().expect("tool metrics");
        assert!(tools.iter().any(|row| {
            row.activity_id == "codex_metrics"
                && row.tool_name == "command_execution"
                && row.call_count == 1
        }));
    }

    #[test]
    fn failing_direct_run_records_failure_state() {
        let (_root, runtime, repo_root, _global_root) = test_runtime();
        let yaml_path = repo_root.join("qa_failing.yaml");
        write_job(&yaml_path, "qa_failing", "missing_action");

        let err = runtime
            .run_job_v2_from_yaml(&yaml_path, json!({}), None)
            .expect_err("direct job run should fail");
        assert!(
            err.to_string()
                .contains("deterministic action not registered"),
            "{err}"
        );

        let history = runtime.job_history("qa_failing").expect("failure history");
        let run = history.first().expect("failed run");
        assert_eq!(run.state, JobRunState::Failed);
        assert!(run.steps.iter().any(|step| {
            step.error_message
                .as_deref()
                .is_some_and(|message| message.contains("deterministic action not registered"))
        }));
        assert!(
            runtime
                .read_run_state(&run.run_id)
                .expect("read run state")
                .is_some()
        );
    }
}
