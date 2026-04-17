use orbit_types::{AgentModelPair, agent_family_from_cli, resolve_agent_model_pair};

use crate::OrbitRuntime;

pub(super) fn normalize_agent_name(agent_cli: &str) -> String {
    std::path::Path::new(agent_cli)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(agent_cli)
        .to_ascii_lowercase()
}

impl OrbitRuntime {
    pub(crate) fn configured_agent_model_pair(&self, agent_cli: &str) -> Option<AgentModelPair> {
        self.stores()
            .executors()
            .get(agent_cli)
            .ok()
            .flatten()
            .and_then(|def| {
                Some(AgentModelPair::new(
                    def.model_for_tier("strong")?.to_string(),
                    def.model_for_tier("weak")?.to_string(),
                ))
            })
            .or_else(|| resolve_agent_model_pair(agent_cli))
    }

    pub(crate) fn canonical_model_for_agent(
        &self,
        agent_cli: &str,
        model: Option<&str>,
    ) -> Option<String> {
        let requested = model.map(str::trim).filter(|value| !value.is_empty())?;
        let pair = self.configured_agent_model_pair(agent_cli);
        let family = agent_family_from_cli(agent_cli);

        if requested.eq_ignore_ascii_case("strong") {
            return pair.map(|pair| pair.orchestrator);
        }
        if requested.eq_ignore_ascii_case("weak") {
            return pair.map(|pair| pair.helper);
        }

        if let Some(pair) = pair {
            if requested.eq_ignore_ascii_case(&pair.orchestrator) {
                return Some(pair.orchestrator);
            }
            if requested.eq_ignore_ascii_case(&pair.helper) {
                return Some(pair.helper);
            }
            if matches_model_alias(&family, requested, &pair.orchestrator, true) {
                return Some(pair.orchestrator);
            }
            if matches_model_alias(&family, requested, &pair.helper, false) {
                return Some(pair.helper);
            }
        }

        Some(requested.to_string())
    }

    pub(crate) fn canonical_agent_model_identity(
        &self,
        agent_cli: Option<&str>,
        model: Option<&str>,
    ) -> (Option<String>, Option<String>) {
        let agent = agent_cli
            .map(normalize_agent_name)
            .filter(|value| !value.trim().is_empty());
        let model = agent
            .as_deref()
            .and_then(|agent| self.canonical_model_for_agent(agent, model))
            .or_else(|| {
                model
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
            });
        (agent, model)
    }
}

fn matches_model_alias(family: &str, requested: &str, configured: &str, strong: bool) -> bool {
    if requested.eq_ignore_ascii_case(configured) {
        return true;
    }

    if let Some(default_pair) = resolve_agent_model_pair(family) {
        let fallback = if strong {
            default_pair.orchestrator
        } else {
            default_pair.helper
        };
        if requested.eq_ignore_ascii_case(&fallback) {
            return true;
        }
    }

    match (family, strong) {
        ("claude", true) => {
            requested.eq_ignore_ascii_case("opus")
                || claude_cli_full_model_name(configured)
                    .is_some_and(|value| requested.eq_ignore_ascii_case(&value))
        }
        ("claude", false) => {
            requested.eq_ignore_ascii_case("sonnet")
                || claude_cli_full_model_name(configured)
                    .is_some_and(|value| requested.eq_ignore_ascii_case(&value))
        }
        ("gemini", true) => requested.eq_ignore_ascii_case("gemini-3.1-pro"),
        ("gemini", false) => requested.eq_ignore_ascii_case("gemini-3-flash"),
        _ => false,
    }
}

fn claude_cli_full_model_name(model: &str) -> Option<String> {
    let trimmed = model.trim();
    if let Some(version) = trimmed.strip_prefix("opus-") {
        return Some(format!("claude-opus-{}", version.replace('.', "-")));
    }
    if let Some(version) = trimmed.strip_prefix("sonnet-") {
        return Some(format!("claude-sonnet-{}", version.replace('.', "-")));
    }
    None
}
