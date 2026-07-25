//! `impl V2RuntimeHost for OrbitRuntime` — the orbit-core side of the v2
//! dispatch boundary.
//!
//! The trait surface is deliberately small: orbit-core owns deterministic
//! action dispatch (which needs the live `ToolContext` + tool registry),
//! provider credential sourcing (env / config access), and the CLI-command
//! resolution for `backend: cli` (workspace-scoped env / config overrides).
//! HTTP agent-loop transport and CLI subprocess execution both live in
//! `orbit-engine`, so this module never names orbit-agent types.

mod backlog_exclusion;
mod cli_executor;
mod dispatch;
mod learning_reminders;
mod pipeline_actions;
mod sandbox;
mod task_context;
#[cfg(test)]
mod test_support;
mod triage;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use orbit_common::types::activity_job::AgentRole;
use orbit_common::types::{
    InvocationTrace, LearningInjectionCaps, LearningInjectionState, LearningReminder, RoleSlot,
    UNRESTRICTED_FS_PROFILE,
};
use orbit_engine::{AgentRoleConfig, EnvironmentHost};
use orbit_engine::{DispatchError, ResolvedCliExecutor, ResolvedSandbox, V2RuntimeHost};
use orbit_store::{InvocationInsertParams, Store, token_scoreboard};
use orbit_tools::{FsAuditLogger, ReservationOwnerContext, ToolContext};
use serde_json::Value;

use crate::OrbitRuntime;
use crate::runtime::build_orbit_tool_host;

impl V2RuntimeHost for OrbitRuntime {
    fn run_deterministic(
        &self,
        action: &str,
        config: &Value,
        input: &Value,
        tool_context: ToolContext,
    ) -> Result<Value, DispatchError> {
        dispatch::run_deterministic(self, action, config, input, tool_context)
    }

    /// [ORB-10385] Report this binary's deterministic-action registry so job
    /// validation can reject a catalog asset naming an action we cannot
    /// dispatch, before the run admits a task or builds a worktree.
    fn has_deterministic_action(&self, action: &str) -> bool {
        dispatch::is_deterministic_action_registered(action)
    }

    fn resolve_cli_executor(&self, provider: &str) -> Result<ResolvedCliExecutor, DispatchError> {
        cli_executor::resolve_cli_executor(self, provider)
    }

    fn provider_cli_config(&self, _provider: &str) -> HashMap<String, String> {
        EnvironmentHost::agent_provider_config(self)
    }

    fn resolve_executor_sandbox(
        &self,
        provider: &str,
        #[cfg(target_os = "macos")] fs_profile: Option<&str>,
        #[cfg(not(target_os = "macos"))] _fs_profile: Option<&str>,
        #[cfg(target_os = "macos")] subprocess_cwd: Option<&Path>,
        #[cfg(not(target_os = "macos"))] _subprocess_cwd: Option<&Path>,
    ) -> Result<Option<ResolvedSandbox>, DispatchError> {
        sandbox::resolve_executor_sandbox(
            self,
            provider,
            #[cfg(target_os = "macos")]
            fs_profile,
            #[cfg(not(target_os = "macos"))]
            _fs_profile,
            #[cfg(target_os = "macos")]
            subprocess_cwd,
            #[cfg(not(target_os = "macos"))]
            _subprocess_cwd,
        )
    }

    fn task_context_for_agent_input(&self, input: &Value) -> Result<Option<Value>, DispatchError> {
        task_context::task_context_for_agent_input(self, input)
    }

    fn learning_reminders_for_task(
        &self,
        input: &Value,
        caps: LearningInjectionCaps,
    ) -> Result<Vec<LearningReminder>, DispatchError> {
        learning_reminders::learning_reminders_for_task(self, input, caps)
    }

    fn persist_session_learning_state(
        &self,
        session_id: &str,
        state: &LearningInjectionState,
    ) -> Result<(), DispatchError> {
        let store = Store::open(&self.context.persistence().audit_db).map_err(|error| {
            DispatchError::JobExecution(format!("open session learning store: {error}"))
        })?;
        let workspace_id = self.workspace_id().map_err(|error| {
            DispatchError::JobExecution(format!("resolve workspace id: {error}"))
        })?;
        store
            .upsert_session_learning_state(&workspace_id, session_id, state)
            .map_err(|error| {
                DispatchError::JobExecution(format!("persist session learning state: {error}"))
            })
    }

    /// [ORB-10002] Persist a per-step checkpoint into the run's
    /// `PipelineState` so an interrupted run can be resumed without
    /// re-executing completed steps. A missing run row (direct `execute_job`
    /// callers that never persisted a run) is a silent no-op — there is
    /// nothing durable to checkpoint into.
    fn checkpoint_step(
        &self,
        run_id: &str,
        step_index: u32,
        step_id: &str,
        output: &Value,
        pipeline_snapshot: &Value,
    ) -> Result<(), DispatchError> {
        let Some(mut state) = self.read_run_state(run_id).map_err(|error| {
            DispatchError::JobExecution(format!("read run state for checkpoint: {error}"))
        })?
        else {
            return Ok(());
        };
        state.record_step(
            step_index,
            orbit_common::types::JobRunState::Success,
            Some(output.clone()),
            None,
        );
        state.sync_pipeline(pipeline_snapshot.clone());
        self.stores()
            .jobs()
            .write_run_state(run_id, &state)
            .map_err(|error| {
                DispatchError::JobExecution(format!(
                    "persist step checkpoint (run {run_id}, step {step_index} `{step_id}`): {error}"
                ))
            })
    }

    fn tool_context_for_activity(
        &self,
        run_id: Option<&str>,
        fs_profile: Option<&str>,
        fs_audit: Option<Arc<dyn FsAuditLogger>>,
        proc_allowed_programs: Option<&[String]>,
    ) -> ToolContext {
        let workspace_root = self
            .paths()
            .repo_root
            .canonicalize()
            .unwrap_or_else(|_| self.paths().repo_root.clone());

        let proc_spawn_activity_scoped = proc_allowed_programs.is_some();
        let proc_allowed_programs = proc_allowed_programs
            .map(|programs| programs.to_vec())
            .unwrap_or_default();

        ToolContext {
            cwd: std::env::current_dir()
                .ok()
                .map(|cwd| cwd.to_string_lossy().into_owned()),
            workspace_root: Some(workspace_root),
            policy_engine: Some(Arc::new(self.policy_engine().clone())),
            fs_profile: Some(fs_profile.unwrap_or(UNRESTRICTED_FS_PROFILE).to_string()),
            fs_audit,
            proc_allowed_programs,
            proc_spawn_activity_scoped,
            reservation_owner: run_id.map(str::trim).filter(|value| !value.is_empty()).map(
                |owner_run_id| ReservationOwnerContext {
                    owner_run_id: owner_run_id.to_string(),
                    owner_metadata_json: Some(
                        serde_json::json!({
                            "source": "v2_activity",
                            "fs_profile": fs_profile.unwrap_or(UNRESTRICTED_FS_PROFILE),
                        })
                        .to_string(),
                    ),
                },
            ),
            orbit_host: Some(build_orbit_tool_host(
                self,
                None,
                run_id
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned),
            )),
            ..Default::default()
        }
    }

    fn persist_invocation_trace(
        &self,
        job_run_id: &str,
        activity_id: &str,
        provider: &str,
        model: Option<&str>,
        input: &Value,
        trace: &InvocationTrace,
    ) -> Result<(), DispatchError> {
        let (agent, model) = self.invocation_agent_model_identity(
            provider,
            model,
            trace.provider_model.as_deref(),
            job_run_id,
            activity_id,
        );
        let store = Store::open(&self.context.persistence().audit_db).map_err(|error| {
            DispatchError::JobExecution(format!("open invocation store: {error}"))
        })?;
        store
            .insert_invocation_trace_record(&InvocationInsertParams {
                job_run_id: job_run_id.to_string(),
                activity_id: activity_id.to_string(),
                agent: agent.unwrap_or_else(|| provider.to_ascii_lowercase()),
                model,
                slot: role_slot_from_input(input),
                task_ids: task_context::associated_task_ids(input),
                trace: trace.clone(),
            })
            .map_err(|error| {
                DispatchError::JobExecution(format!("persist invocation trace: {error}"))
            })?;

        if let Err(error) =
            token_scoreboard::write_token_scoreboard(&self.paths().scoreboard_dir, &store)
        {
            tracing::warn!(
                target: "orbit.core.scoreboard",
                error = %error,
                "failed to refresh tokens scoreboard",
            );
        }

        let existing = self
            .get_job_run_backend(job_run_id)
            .map_err(|error| {
                DispatchError::JobExecution(format!("read job run for knowledge metrics: {error}"))
            })?
            .and_then(|run| run.knowledge_metrics);
        if let Some(metrics) = crate::metrics::merge_invocation_trace(existing.as_ref(), trace) {
            self.stores()
                .jobs()
                .record_run_knowledge_metrics(job_run_id, metrics)
                .map_err(|error| {
                    DispatchError::JobExecution(format!(
                        "record job-run knowledge metrics: {error}"
                    ))
                })?;
        }

        Ok(())
    }

    fn agent_role_config(&self, role: AgentRole) -> Option<AgentRoleConfig> {
        EnvironmentHost::agent_role_config(self, role)
    }

    fn agent_role_config_for_input(
        &self,
        role: AgentRole,
        input: &serde_json::Value,
    ) -> Option<AgentRoleConfig> {
        let crew = self
            .resolve_crew_for_run_input(input)
            .map_err(|error| {
                tracing::warn!(
                    target: "orbit.config.crew",
                    error = %error,
                    "failed to resolve crew for activity input; falling back to default role config",
                );
                error
            })
            .ok()?;
        let assignment = crew.role(role.as_str())?;
        Some(
            crate::runtime::engine::environment_host::typed_role_config_from_assignment(
                role, assignment,
            ),
        )
    }

    fn explicit_agent_crew_config_for_input(
        &self,
        input: &serde_json::Value,
    ) -> Result<Option<AgentRoleConfig>, DispatchError> {
        let Some(explicit) = input
            .get("crew")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };
        let crew = self
            .resolve_crew_for_task(Some(explicit), None)
            .map_err(|error| {
                DispatchError::JobValidation(format!(
                    "explicit activity crew '{explicit}' cannot be resolved: {error}"
                ))
            })?;
        Ok(Some(
            crate::runtime::engine::environment_host::typed_role_config_from_assignment(
                AgentRole::Reviewer,
                &crew.assignment,
            ),
        ))
    }

    fn api_key_for(&self, provider: &str) -> Result<String, DispatchError> {
        match provider {
            "anthropic" => {
                let key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
                    DispatchError::AgentLoopFailed(
                        "ANTHROPIC_API_KEY not set — export it before running a v2 agent_loop activity"
                            .to_string(),
                    )
                })?;
                if key.is_empty() {
                    return Err(DispatchError::AgentLoopFailed(
                        "ANTHROPIC_API_KEY is empty".to_string(),
                    ));
                }
                Ok(key)
            }
            other => Err(DispatchError::AgentLoopFailed(format!(
                "unsupported provider: {other}"
            ))),
        }
    }
}

fn role_slot_from_input(input: &Value) -> Option<RoleSlot> {
    input
        .get("planning_duel_slot")
        .or_else(|| input.get("role_slot"))
        .or_else(|| input.get("slot"))
        .and_then(Value::as_str)
        .and_then(|value| value.parse().ok())
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use orbit_common::types::activity_job::{AgentLoopSpec, Backend, OnDenial, Provider};
    use orbit_common::types::{
        InvocationTrace, JobRunState, TaskPriority, TaskStatus, TaskType, TokenUsage, ToolCallTrace,
    };
    use orbit_engine::{V2AuditWriter, drive_agent_loop, reset_replay_transport};
    use orbit_store::InvocationQuery;
    use tempfile::NamedTempFile;

    use super::test_support::{runtime_with_workspace_layout, seed_list_backlog_task};
    use super::*;

    fn replay_env_guard() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    struct ReplayFixtureGuard {
        prior: Option<String>,
    }

    impl ReplayFixtureGuard {
        fn set(path: &std::path::Path) -> Self {
            let prior = std::env::var("ORBIT_V2_REPLAY_FIXTURE").ok();
            // SAFETY: replay fixture env mutation is serialized by `replay_env_guard`.
            unsafe {
                std::env::set_var("ORBIT_V2_REPLAY_FIXTURE", path);
            }
            reset_replay_transport();
            Self { prior }
        }
    }

    impl Drop for ReplayFixtureGuard {
        fn drop(&mut self) {
            reset_replay_transport();
            // SAFETY: replay fixture env mutation is serialized by `replay_env_guard`.
            unsafe {
                match &self.prior {
                    Some(value) => std::env::set_var("ORBIT_V2_REPLAY_FIXTURE", value),
                    None => std::env::remove_var("ORBIT_V2_REPLAY_FIXTURE"),
                }
            }
        }
    }

    fn write_replay_fixture(value: Value) -> NamedTempFile {
        let file = NamedTempFile::new().expect("fixture temp file");
        std::fs::write(
            file.path(),
            serde_json::to_vec(&value).expect("serialize replay fixture"),
        )
        .expect("write replay fixture");
        file
    }

    fn seed_running_job_run(runtime: &OrbitRuntime, job_id: &str) -> String {
        let run = runtime
            .stores()
            .jobs()
            .insert_run(job_id, 1, chrono::Utc::now(), None, None)
            .expect("insert job run");
        runtime
            .stores()
            .jobs()
            .mark_run_running(&run.run_id, chrono::Utc::now(), std::process::id())
            .expect("mark run running");
        run.run_id
    }

    fn payload_tool_call(seq: u32, tool_name: &str, payload: Value) -> ToolCallTrace {
        ToolCallTrace {
            seq,
            tool_name: tool_name.to_string(),
            result_bytes: serde_json::to_vec(&payload)
                .expect("serialize payload")
                .len() as u64,
            result_payload: Some(payload),
        }
    }

    fn byte_count_tool_call(seq: u32, tool_name: &str, result_bytes: u64) -> ToolCallTrace {
        ToolCallTrace {
            seq,
            tool_name: tool_name.to_string(),
            result_bytes,
            result_payload: None,
        }
    }

    fn trace_with_tool_calls(input_tokens: u64, tool_calls: Vec<ToolCallTrace>) -> InvocationTrace {
        InvocationTrace {
            usage: TokenUsage {
                input: input_tokens,
                cache_read: 0,
                cache_create: 0,
                cache_create_1h: 0,
                output: 0,
            },
            tool_calls,
            duration_ms: 10,
            provider_model: None,
            provider_cost_usd: None,
        }
    }

    #[test]
    fn persist_invocation_trace_prefers_provider_model_over_requested_alias() {
        let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
        let run_id = seed_running_job_run(&runtime, "provider_model_job");
        let trace = InvocationTrace {
            provider_model: Some("claude-fable-5".to_string()),
            ..InvocationTrace::default()
        };

        V2RuntimeHost::persist_invocation_trace(
            &runtime,
            &run_id,
            "implement_one",
            "claude",
            Some("fable"),
            &serde_json::json!({ "task_id": "ORB-10370" }),
            &trace,
        )
        .expect("persist provider model");

        let records = runtime
            .invocation_records(InvocationQuery {
                job_run_id: Some(run_id),
                limit: 1,
                ..InvocationQuery::default()
            })
            .expect("query invocation records");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].agent, "claude");
        assert_eq!(records[0].model.as_deref(), Some("claude-fable-5"));
    }

    fn persist_test_trace(runtime: &OrbitRuntime, run_id: &str, trace: &InvocationTrace) {
        V2RuntimeHost::persist_invocation_trace(
            runtime,
            run_id,
            "knowledge_step",
            "codex",
            Some("gpt-test"),
            &serde_json::json!({ "task_id": "ORB-KNOWLEDGE-TEST" }),
            trace,
        )
        .expect("persist invocation trace");
    }

    #[test]
    fn persist_invocation_trace_no_longer_measures_removed_pack_tool() {
        // ORB-00391: orbit.graph.pack was removed with orbit-knowledge (v1). A trace
        // whose only payload tool is the former pack tool records no knowledge metrics,
        // because merge_invocation_trace now measures fs.read exclusively.
        let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
        let run_id = seed_running_job_run(&runtime, "knowledge_pack_job");
        let trace = trace_with_tool_calls(
            155,
            vec![payload_tool_call(
                1,
                "orbit.graph.pack",
                serde_json::json!({
                    "raw_read_token_baseline": 400,
                    "knowledge_pack_tokens": 100,
                    "entries": [{ "selector": "file:src/lib.rs", "source": "pub fn demo() {}" }],
                    "unresolved_selectors": [],
                }),
            )],
        );

        persist_test_trace(&runtime, &run_id, &trace);

        let run = runtime.show_job_run(&run_id).expect("show job run");
        assert_eq!(run.state, JobRunState::Running);
        assert!(
            run.knowledge_metrics.is_none(),
            "the removed pack tool must not produce knowledge metrics"
        );
        assert_eq!(run.job_id, "knowledge_pack_job");
    }

    #[test]
    fn persist_invocation_trace_records_fs_read_double_read_metrics() {
        // ORB-00391: with the pack baseline gone, every fs.read is "double read"
        // relative to itself, so double_read_rate is 1.0 for an fs.read-only run.
        let (_root, runtime, _repo_root) = runtime_with_workspace_layout();

        let fallback_run_id = seed_running_job_run(&runtime, "knowledge_fallback_job");
        let fallback_trace =
            trace_with_tool_calls(50, vec![byte_count_tool_call(1, "fs.read", 120)]);

        persist_test_trace(&runtime, &fallback_run_id, &fallback_trace);

        let fallback_run = runtime
            .show_job_run(&fallback_run_id)
            .expect("show fallback job run");
        let metrics = fallback_run
            .knowledge_metrics
            .expect("fallback metrics recorded");
        assert!(!metrics.knowledge_pack_used);
        assert_eq!(metrics.raw_read_token_baseline, 30);
        assert_eq!(metrics.knowledge_pack_tokens, None);
        assert_eq!(metrics.actual_fs_read_tokens_during_run, 30);
        assert_eq!(metrics.double_read_rate, Some(1.0));
        assert_eq!(metrics.total_llm_input_tokens, 50);
    }

    #[test]
    fn http_agent_loop_tool_update_persists_runtime_identity_family() {
        let _lock = replay_env_guard();
        let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
        let task = seed_list_backlog_task(
            &runtime,
            "runtime identity regression",
            TaskStatus::InProgress,
            TaskPriority::Medium,
            TaskType::Chore,
            None,
            Vec::new(),
        );
        let fixture = write_replay_fixture(serde_json::json!({
            "turns": [
                {
                    "content": [{
                        "kind": "tool_use",
                        "id": "toolu_identity_update",
                        "name": "orbit.task.update",
                        "input": {
                            "id": task.id.clone(),
                            "status": "review",
                            "execution_summary": "Identity regression covered.",
                            "model": "grok-build"
                        }
                    }],
                    "stop_reason": "tool_use"
                },
                {
                    "content": [{ "kind": "text", "text": "done" }],
                    "stop_reason": "end_turn"
                }
            ]
        }));
        let _guard = ReplayFixtureGuard::set(fixture.path());
        let audit_dir = tempfile::tempdir().expect("audit tempdir");
        let audit = V2AuditWriter::with_disk_sinks(
            audit_dir.path(),
            Store::open_in_memory().expect("audit store"),
            "ws_test",
            "http-identity-regression",
            format!("claude:{}", orbit_common::test_fixtures::TEST_CLAUDE_MODEL),
            None,
        )
        .expect("audit writer");
        let spec = AgentLoopSpec {
            instruction: "exercise tool identity".to_string(),
            tools: vec!["orbit.task.update".to_string()],
            on_denial: OnDenial::Terminate,
            model: Some(orbit_common::test_fixtures::TEST_CLAUDE_MODEL.to_string()),
            max_iterations: 2,
            backend: Backend::Http,
            provider: Provider::Claude,
            wall_clock_timeout_seconds: 30,
            require_response_envelope: false,
            role: None,
            proc_allowed_programs: None,
        };

        drive_agent_loop(
            &spec,
            None,
            "http-identity-regression",
            audit,
            &serde_json::json!({ "prompt": "update the task" }),
            &runtime,
            None,
        )
        .expect("replay agent loop succeeds");

        let updated = runtime.get_task(&task.id).expect("updated task");
        assert_eq!(updated.implemented_by.as_deref(), Some("claude"));
    }

    #[test]
    fn tool_context_for_activity_passes_proc_allowlist() {
        let (_root, runtime, _repo_root) = runtime_with_workspace_layout();

        // No allowlist -> not activity-scoped (legacy unrestricted path).
        let unscoped = <OrbitRuntime as V2RuntimeHost>::tool_context_for_activity(
            &runtime,
            Some("run-allowlist-test"),
            None,
            None,
            None,
        );
        assert!(unscoped.proc_allowed_programs.is_empty());
        assert!(!unscoped.proc_spawn_activity_scoped);

        // Activity-scoped allowlist propagates verbatim and flips the bool.
        let programs = vec!["git".to_string(), "rg".to_string()];
        let scoped = <OrbitRuntime as V2RuntimeHost>::tool_context_for_activity(
            &runtime,
            Some("run-allowlist-test"),
            None,
            None,
            Some(programs.as_slice()),
        );
        assert_eq!(scoped.proc_allowed_programs, programs);
        assert!(scoped.proc_spawn_activity_scoped);

        // Empty Some([]) is meaningful: fail-closed when activity-scoped.
        let empty_scoped = <OrbitRuntime as V2RuntimeHost>::tool_context_for_activity(
            &runtime,
            Some("run-allowlist-test"),
            None,
            None,
            Some(&[]),
        );
        assert!(empty_scoped.proc_allowed_programs.is_empty());
        assert!(empty_scoped.proc_spawn_activity_scoped);
    }
}
