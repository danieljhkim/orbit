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

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use orbit_common::types::activity_job::AgentRole;
use orbit_common::types::{
    InvocationTrace, LearningInjectionCaps, LearningReminder, RoleSlot, UNRESTRICTED_FS_PROFILE,
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
    use orbit_common::types::{TaskPriority, TaskStatus, TaskType};
    use orbit_engine::{V2AuditWriter, drive_agent_loop, reset_replay_transport};
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
            "http-identity-regression",
            "claude:claude-opus-4-7".to_string(),
            None,
        )
        .expect("audit writer");
        let spec = AgentLoopSpec {
            instruction: "exercise tool identity".to_string(),
            tools: vec!["orbit.task.update".to_string()],
            on_denial: OnDenial::Terminate,
            model: Some("claude-opus-4-7".to_string()),
            max_iterations: 2,
            backend: Backend::Http,
            provider: Provider::Claude,
            wall_clock_timeout_seconds: 30,
            role: None,
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
}
