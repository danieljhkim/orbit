//! `impl V2RuntimeHost for OrbitRuntime` — the orbit-core side of the v2
//! dispatch boundary.
//!
//! The trait surface is deliberately small: orbit-core owns deterministic
//! action dispatch (which needs the live `ToolContext` + tool registry),
//! provider credential sourcing (env / config access), and the CLI-command
//! resolution for `backend: cli` (workspace-scoped env / config overrides).
//! HTTP agent-loop transport and CLI subprocess execution both live in
//! `orbit-engine`, so this module never names orbit-agent types.

use std::sync::Arc;
use std::time::{Duration, Instant};

use orbit_engine::v2::{DispatchError, V2RuntimeHost};
use orbit_tools::{FsAuditLogger, ToolContext};
use orbit_types::{Role, UNRESTRICTED_FS_PROFILE};
use serde_json::Value;

use super::orbit_tool_host::{
    emit_expired_reservation_events, merge_task_lock_conflicts, parse_task_ids,
    requested_task_files, task_lock_conflicts, workspace_orbit_dir,
};
use crate::OrbitRuntime;

impl V2RuntimeHost for OrbitRuntime {
    fn run_deterministic(
        &self,
        action: &str,
        config: &Value,
        input: &Value,
        tool_context: ToolContext,
    ) -> Result<Value, DispatchError> {
        match action {
            "orbit_tool_call" => {
                // The `config` block shape (see v2_deterministic_reference.yaml):
                //   config: { tool_name: <name>, args: <object> }
                // Input overrides config when both are present.
                let tool_name = input
                    .get("tool_name")
                    .or_else(|| config.get("tool_name"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: "missing `tool_name` in config or input".to_string(),
                    })?;
                let args = input
                    .get("args")
                    .or_else(|| config.get("args"))
                    .cloned()
                    .unwrap_or(Value::Null);

                self.run_tool_with_context_and_role(tool_name, args, Role::Admin, tool_context)
                    .map_err(|err| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: format!("{err}"),
                    })
            }
            // Phase 4 stub handlers. Real git/API logic lands in a follow-up
            // task once the per-asset migration ports the rest of the
            // pipeline dependencies (worktree_setup, pr_open, pr_merge, …).
            // Returning a structured result keeps the activities dispatchable
            // so the §7 `activity.started` / `activity.finished` envelope is
            // emitted end-to-end — an operator running the pipeline today
            // sees the intent even while the implementation is stubbed.
            "promote_agent_main" => {
                let target = input
                    .get("target_branch")
                    .and_then(Value::as_str)
                    .unwrap_or("main");
                let source = input
                    .get("source_branch")
                    .and_then(Value::as_str)
                    .unwrap_or("agent-main");
                Ok(serde_json::json!({
                    "promoted": false,
                    "target_sha": null,
                    "skipped_reason":
                        format!("stub: real promotion from `{source}` to `{target}` lands in a follow-up"),
                }))
            }
            "revert_on_red" => {
                let sha = input
                    .get("commit_sha")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                Ok(serde_json::json!({
                    "reverted": false,
                    "revert_sha": null,
                    "follow_up_issue": null,
                    "skipped_reason":
                        format!("stub: real revert of `{sha}` lands in a follow-up"),
                }))
            }
            "context_conflict_check" => {
                let task_ids = parse_task_ids(input).map_err(|error| {
                    DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: error.to_string(),
                    }
                })?;
                let requested_files = requested_task_files(self, &task_ids).map_err(|error| {
                    DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: error.to_string(),
                    }
                })?;
                let task_conflicts = task_lock_conflicts(self, &task_ids, &requested_files)
                    .map_err(|error| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: error.to_string(),
                    })?;
                let reservation_check = self
                    .stores()
                    .task_reservations()
                    .check(orbit_store::TaskReservationCheckParams {
                        workspace_orbit_dir: workspace_orbit_dir(self),
                        requested_files,
                    })
                    .map_err(|error| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: error.to_string(),
                    })?;
                emit_expired_reservation_events(self, &reservation_check.expired_reservations)
                    .map_err(|error| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: error.to_string(),
                    })?;
                let conflicts =
                    merge_task_lock_conflicts(task_conflicts, reservation_check.conflicts);
                Ok(serde_json::json!({
                    "clear": conflicts.is_empty(),
                    "conflicts": conflicts,
                }))
            }
            "sleep" => {
                let seconds = input
                    .get("seconds")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: "missing `seconds`".to_string(),
                    })?;
                if !(0.0..=3600.0).contains(&seconds) {
                    return Err(DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: "`seconds` must be between 0 and 3600".to_string(),
                    });
                }
                let started_at = Instant::now();
                std::thread::sleep(Duration::from_secs_f64(seconds));
                Ok(serde_json::json!({
                    "slept_seconds": started_at.elapsed().as_secs_f64(),
                }))
            }
            other => Err(DispatchError::DeterministicActionNotRegistered(
                other.to_string(),
            )),
        }
    }

    fn resolve_cli_command(&self, provider: &str) -> Result<String, DispatchError> {
        resolve_cli_command(provider)
    }

    fn tool_context_for_activity(
        &self,
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
            ..Default::default()
        }
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

/// Map a v2 provider name to the CLI command that dispatches it. Env-var
/// overrides (`ORBIT_V2_CLI_<PROVIDER>`) let smokes substitute a fixture
/// binary for the real provider CLI; production defaults to the provider
/// name itself (`claude`, `codex`, `gemini`, `ollama`) which resolves via
/// `$PATH`.
fn resolve_cli_command(provider: &str) -> Result<String, DispatchError> {
    let env_key = format!("ORBIT_V2_CLI_{}", provider.to_ascii_uppercase());
    if let Ok(value) = std::env::var(&env_key) {
        if !value.is_empty() {
            return Ok(value);
        }
    }
    match provider {
        "claude" | "codex" | "gemini" | "ollama" => Ok(provider.to_string()),
        "openai_compat" => Err(DispatchError::CliInvocationFailed(
            "provider openai_compat has no CLI runtime (HTTP-only)".to_string(),
        )),
        other => Err(DispatchError::CliInvocationFailed(format!(
            "unknown provider `{other}` — no CLI runtime registered"
        ))),
    }
}
