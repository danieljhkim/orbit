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
mod pipeline_actions;
mod sandbox;
mod task_context;
#[cfg(test)]
mod test_support;

use std::collections::HashMap;
use std::sync::Arc;

use orbit_common::types::activity_job::AgentRole;
use orbit_common::types::{InvocationTrace, UNRESTRICTED_FS_PROFILE};
use orbit_engine::activity_job::{
    DispatchError, ResolvedCliExecutor, ResolvedSandbox, V2RuntimeHost,
};
use orbit_engine::{AgentRoleConfig, EnvironmentHost};
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
    ) -> Result<Option<ResolvedSandbox>, DispatchError> {
        sandbox::resolve_executor_sandbox(
            self,
            provider,
            #[cfg(target_os = "macos")]
            fs_profile,
            #[cfg(not(target_os = "macos"))]
            _fs_profile,
        )
    }

    fn task_context_for_agent_input(&self, input: &Value) -> Result<Option<Value>, DispatchError> {
        task_context::task_context_for_agent_input(self, input)
    }

    fn tool_context_for_activity(
        &self,
        run_id: Option<&str>,
        fs_profile: Option<&str>,
        fs_audit: Option<Arc<dyn FsAuditLogger>>,
    ) -> ToolContext {
        let workspace_root = self
            .paths()
            .repo_root
            .canonicalize()
            .unwrap_or_else(|_| self.paths().repo_root.clone());

        ToolContext {
            cwd: std::env::current_dir()
                .ok()
                .map(|cwd| cwd.to_string_lossy().into_owned()),
            workspace_root: Some(workspace_root),
            policy_engine: Some(Arc::new(self.policy_engine().clone())),
            fs_profile: Some(fs_profile.unwrap_or(UNRESTRICTED_FS_PROFILE).to_string()),
            fs_audit,
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
            orbit_host: Some(build_orbit_tool_host(self, None)),
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
        let (agent, model) = self.canonical_agent_model_identity(Some(provider), model);
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

        Ok(())
    }

    fn agent_role_config(&self, role: AgentRole) -> Option<AgentRoleConfig> {
        EnvironmentHost::agent_role_config(self, role)
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
