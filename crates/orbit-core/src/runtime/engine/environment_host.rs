use orbit_common::types::OrbitError;
use orbit_common::types::activity_job::{AgentRole, Backend, Provider};
use orbit_engine::{AgentRoleConfig, EnvironmentHost, ExecutorLookupHost};

use super::paths::codex_workspace_write_writable_dirs;
use crate::OrbitRuntime;
use crate::config::RawAgentRoleConfig;

impl EnvironmentHost for OrbitRuntime {
    fn agent_provider_config(&self) -> std::collections::HashMap<String, String> {
        let mut config = std::collections::HashMap::new();
        let policy = self.codex_execution_policy();
        config.insert("sandbox".to_string(), policy.sandbox().to_string());
        if let Some(approval) = policy.approval_policy() {
            config.insert("approval_policy".to_string(), approval.to_string());
        }
        if policy.sandbox() == "workspace-write" {
            config.insert(
                "writable_dirs_json".to_string(),
                serde_json::to_string(&codex_workspace_write_writable_dirs(self.context.paths()))
                    .unwrap_or_else(|_| "[]".to_string()),
            );
        }
        config
    }

    fn execution_env_inherit(&self) -> bool {
        self.execution_env_policy().inherit()
    }

    fn hydrated_env_allowlist(&self, env_extra: &[String]) -> Vec<(String, String)> {
        self.execution_env_policy()
            .hydrated_allowlist_env_with_extras(env_extra)
    }

    fn orbit_root(&self) -> Option<String> {
        Some(
            self.context
                .paths()
                .orbit_dir
                .to_string_lossy()
                .into_owned(),
        )
    }

    fn cli_command_environment(&self, env_extra: &[String]) -> Vec<(String, String)> {
        self.execution_env_policy()
            .hydrated_cli_command_env_with_extras(env_extra)
    }

    fn missing_required_environment_vars(&self, required_env_vars: &[&str]) -> Vec<String> {
        self.execution_env_policy()
            .missing_required(required_env_vars)
    }

    fn agent_role_config(&self, role: AgentRole) -> Option<AgentRoleConfig> {
        let raw = self.context.agent_role(role.as_str())?;
        Some(typed_role_config_from_raw(role, raw))
    }
}

/// Convert the on-disk `[agent.<role>]` block (string fields) into the typed
/// [`AgentRoleConfig`] surface used by the engine resolver. Unrecognized
/// `provider` / `backend` values yield `None` for that field with a warn-log
/// — silently coercing dispatch onto a different runtime would defeat the
/// point of the override.
fn typed_role_config_from_raw(role: AgentRole, raw: &RawAgentRoleConfig) -> AgentRoleConfig {
    let provider = raw.provider.as_deref().and_then(|raw_value| {
        let parsed = parse_provider(raw_value);
        if parsed.is_none() {
            tracing::warn!(
                target: "orbit.config.agent_role",
                role = role.as_str(),
                raw = raw_value,
                "[agent.<role>].provider has an unrecognized value; falling back to inline activity provider",
            );
        }
        parsed
    });

    let backend = raw.backend.as_deref().and_then(|raw_value| {
        let parsed = Backend::parse(raw_value);
        if parsed.is_none() {
            tracing::warn!(
                target: "orbit.config.agent_role",
                role = role.as_str(),
                raw = raw_value,
                "[agent.<role>].backend has an unrecognized value; falling back to inline activity backend",
            );
        }
        parsed
    });

    let model = raw
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    AgentRoleConfig {
        provider,
        model,
        backend,
    }
}

fn parse_provider(raw: &str) -> Option<Provider> {
    match raw.trim() {
        "claude" => Some(Provider::Claude),
        "codex" => Some(Provider::Codex),
        "gemini" => Some(Provider::Gemini),
        "ollama" => Some(Provider::Ollama),
        "openai_compat" | "openai-compat" => Some(Provider::OpenaiCompat),
        _ => None,
    }
}

impl ExecutorLookupHost for OrbitRuntime {
    fn get_executor_def(
        &self,
        name: &str,
    ) -> Result<Option<orbit_common::types::ExecutorDef>, OrbitError> {
        self.stores().executors().get(name)
    }
}
