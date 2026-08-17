use std::collections::HashMap;

use orbit_common::OrbitError;
use orbit_types::telemetry::InvocationTrace;

use crate::agent::{AgentConfig, ProviderOptions};
use crate::providers::mock_agent::mock_agent_cli::MockAgentCliTransport;
use crate::runtime::{AgentRuntime, AgentRuntimeFactory};
use crate::types::{AgentInvocationSpec, AgentRequest};

const RUNTIME_KEY: &str = "mock-agent";
const REQUIRED_ENV_VARS: &[&str] = &[];

pub(crate) struct MockAgentRuntime {
    command: String,
    cli: MockAgentCliTransport,
    runtime_key: &'static str,
    required_env_vars: &'static [&'static str],
}

pub(crate) struct MockAgentFactory;

impl MockAgentRuntime {
    pub(crate) fn new(
        command: String,
        runtime_key: &'static str,
        required_env_vars: &'static [&'static str],
    ) -> Self {
        Self {
            command,
            cli: MockAgentCliTransport,
            runtime_key,
            required_env_vars,
        }
    }
}

impl AgentRuntimeFactory for MockAgentFactory {
    fn key(&self) -> &'static str {
        RUNTIME_KEY
    }

    fn required_env_vars(&self) -> &'static [&'static str] {
        REQUIRED_ENV_VARS
    }

    fn options_from_config(
        &self,
        _config: &HashMap<String, String>,
    ) -> Result<ProviderOptions, OrbitError> {
        Ok(ProviderOptions::Mock)
    }

    fn build(&self, cfg: &AgentConfig) -> Result<Box<dyn AgentRuntime>, OrbitError> {
        match &cfg.provider_options {
            ProviderOptions::Mock => Ok(Box::new(MockAgentRuntime::new(
                cfg.command.clone(),
                self.key(),
                self.required_env_vars(),
            ))),
            _ => Err(OrbitError::InvalidInput(format!(
                "provider options '{}' cannot build mock-agent runtime",
                cfg.provider_key
            ))),
        }
    }
}

impl AgentRuntime for MockAgentRuntime {
    fn invoke(
        &self,
        req: AgentRequest,
    ) -> Result<(AgentInvocationSpec, InvocationTrace), OrbitError> {
        Ok((
            crate::providers::build_invocation_spec(
                self.runtime_key,
                self.required_env_vars,
                self.command.clone(),
                self.cli.args(&req.operation),
                self.cli.stdin(&req.envelope_json),
            ),
            InvocationTrace::default(),
        ))
    }

    fn model_name(&self) -> Option<&str> {
        None
    }
}
