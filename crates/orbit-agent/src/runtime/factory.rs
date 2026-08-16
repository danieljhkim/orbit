use std::collections::HashMap;

use orbit_common::OrbitError;

use crate::agent::{AgentConfig, ProviderOptions};
use crate::runtime::{AgentRuntime, ProviderRegistry};

pub trait AgentRuntimeFactory: Send + Sync {
    fn key(&self) -> &'static str;
    fn required_env_vars(&self) -> &'static [&'static str];
    fn options_from_config(
        &self,
        config: &HashMap<String, String>,
    ) -> Result<ProviderOptions, OrbitError>;
    fn build(&self, cfg: &AgentConfig) -> Result<Box<dyn AgentRuntime>, OrbitError>;
}

pub(crate) fn resolve_runtime(
    registry: &ProviderRegistry,
    cfg: &AgentConfig,
) -> Result<Box<dyn AgentRuntime>, OrbitError> {
    let key = cfg.provider_key;
    registry
        .get(key)
        .ok_or_else(|| OrbitError::UnsupportedAgentProvider(key.to_string()))?
        .build(cfg)
}
