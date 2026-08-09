use orbit_common::types::CrewAssignment;
use orbit_common::types::activity_job::{Backend, Provider};
use orbit_engine::{CrewConfig, EnvironmentHost};

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
}

/// Convert a crew assignment (string fields) into the typed [`CrewConfig`]
/// surface used by the engine resolver. Unrecognized
/// `provider` / `backend` values yield `None` for that field with a warn-log
/// — silently coercing dispatch onto a different runtime would defeat the
/// point of the override.
pub(crate) fn typed_crew_config_from_assignment(raw: &CrewAssignment) -> CrewConfig {
    let provider = Some(raw.provider.as_str()).and_then(|raw_value| {
        // Canonical string→provider parsing lives on the orbit-common `Provider`
        // surface (ORB-10091); the crew path routes through it so casing/alias
        // handling cannot drift from the other layers. `resolve_name` preserves
        // the deprecation signal so a legacy alias resolves *and* warns.
        match Provider::resolve_name(raw_value) {
            Ok(identity) => {
                if let Some(deprecation) = identity.deprecation {
                    tracing::warn!(
                        target: "orbit.config.crew",
                        alias = %deprecation.alias,
                        canonical = %deprecation.canonical,
                        "[crews.<name>].provider uses a deprecated alias; resolving to the canonical id — update the config",
                    );
                }
                Some(identity.provider)
            }
            Err(_) => {
                tracing::warn!(
                    target: "orbit.config.crew",
                    raw = raw_value,
                    "[crews.<name>].provider has an unrecognized value; falling back to inline activity provider",
                );
                None
            }
        }
    });

    let backend = Some(raw.backend.as_str()).and_then(|raw_value| {
        let parsed = Backend::parse(raw_value);
        if parsed.is_none() {
            tracing::warn!(
                target: "orbit.config.crew",
                raw = raw_value,
                "[crews.<name>].backend has an unrecognized value; falling back to inline activity backend",
            );
        }
        parsed
    });

    let model = raw.model.trim();
    let model = (!model.is_empty()).then(|| model.to_string());

    CrewConfig {
        provider,
        model,
        backend,
    }
}
