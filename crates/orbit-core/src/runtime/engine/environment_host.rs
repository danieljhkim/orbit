use orbit_common::types::activity_job::{AgentRole, Backend, Provider};
use orbit_common::types::{CrewRoleAssignment, OrbitError};
use orbit_engine::{AgentRoleConfig, EnvironmentHost, ExecutorLookupHost};

use super::paths::codex_workspace_write_writable_dirs;
use crate::OrbitRuntime;

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
        let crew = self
            .context
            .default_crew()
            .and_then(|name| self.context.crews().get(name))?;
        let raw = crew.role(role.as_str())?;
        Some(typed_role_config_from_assignment(role, raw))
    }
}

/// Convert a crew role assignment (string fields) into the typed
/// [`AgentRoleConfig`] surface used by the engine resolver. Unrecognized
/// `provider` / `backend` values yield `None` for that field with a warn-log
/// — silently coercing dispatch onto a different runtime would defeat the
/// point of the override.
pub(crate) fn typed_role_config_from_assignment(
    role: AgentRole,
    raw: &CrewRoleAssignment,
) -> AgentRoleConfig {
    let provider = Some(raw.provider.as_str()).and_then(|raw_value| {
        let parsed = parse_provider(raw_value);
        if parsed.is_none() {
            tracing::warn!(
                target: "orbit.config.crew",
                role = role.as_str(),
                raw = raw_value,
                "[crews.<name>].provider has an unrecognized value; falling back to inline activity provider",
            );
        }
        parsed
    });

    let backend = Some(raw.backend.as_str()).and_then(|raw_value| {
        let parsed = Backend::parse(raw_value);
        if parsed.is_none() {
            tracing::warn!(
                target: "orbit.config.crew",
                role = role.as_str(),
                raw = raw_value,
                "[crews.<name>].backend has an unrecognized value; falling back to inline activity backend",
            );
        }
        parsed
    });

    let model = raw.model.trim();
    let model = (!model.is_empty()).then(|| model.to_string());

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
        "grok" => Some(Provider::Grok),
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
