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
mod task_pilot;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
mod triage;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use orbit_common::types::activity_job::AgentRole;
use orbit_common::types::{
    InvocationTrace, LearningInjectionCaps, LearningInjectionState, LearningReminder,
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
                .record_job_run_knowledge_metrics(job_run_id, metrics)
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

    fn system_crew_for_dispatch(&self) -> Option<String> {
        Some(self.context.settings().system_crew().to_string())
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
        let config_key = input
            .get("crew_config_key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let (crew_name, config_key) = match config_key {
            Some("workflow.system_crew") => (
                self.context.settings().system_crew(),
                Some("workflow.system_crew"),
            ),
            Some(other) => {
                return Err(DispatchError::JobValidation(format!(
                    "explicit activity crew `{explicit}` names unsupported configuration key `{other}`"
                )));
            }
            None => (explicit, None),
        };
        let crew = self
            .resolve_crew_for_task(Some(crew_name), None)
            .map_err(|error| {
                let source = config_key
                    .map(|key| format!("configured by `{key}`"))
                    .unwrap_or_else(|| "from activity input".to_string());
                DispatchError::JobValidation(format!(
                    "explicit activity crew `{crew_name}` ({source}) cannot be resolved or used: {error}"
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
