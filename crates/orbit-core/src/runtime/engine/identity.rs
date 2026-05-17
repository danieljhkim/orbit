use orbit_common::types::{AgentModelPair, OrbitError, normalize_agent_family_for_model};

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
                def.model_pair_override()
                    .map(|pair| AgentModelPair::new(pair.strong.clone(), pair.weak.clone()))
            })
    }

    pub(crate) fn canonical_agent_model_identity(
        &self,
        agent_cli: Option<&str>,
        model: Option<&str>,
    ) -> (Option<String>, Option<String>) {
        self.try_canonical_agent_model_identity(agent_cli, model)
            .unwrap_or_else(|_| self.legacy_canonical_agent_model_identity(agent_cli, model))
    }

    pub(crate) fn try_canonical_agent_model_identity(
        &self,
        agent_cli: Option<&str>,
        model: Option<&str>,
    ) -> Result<(Option<String>, Option<String>), OrbitError> {
        let agent = normalize_agent_family_for_model(agent_cli, model)?;
        let model = model
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        Ok((agent, model))
    }

    fn legacy_canonical_agent_model_identity(
        &self,
        agent_cli: Option<&str>,
        model: Option<&str>,
    ) -> (Option<String>, Option<String>) {
        let agent = agent_cli
            .map(normalize_agent_name)
            .filter(|value| !value.trim().is_empty());
        let model = model
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        (agent, model)
    }
}
