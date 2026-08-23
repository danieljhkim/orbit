use std::collections::HashMap;

use orbit_common::OrbitError;
use orbit_types::telemetry::InvocationTrace;

use crate::agent::{AgentConfig, ProviderOptions};
use crate::providers::cursor::cursor_cli::CursorCliTransport;
use crate::runtime::{AgentRuntime, AgentRuntimeFactory};
use crate::types::{AgentInvocationSpec, AgentRequest};

const RUNTIME_KEY: &str = "cursor-agent";

/// Non-secret process context required by the local Cursor CLI.
///
/// `CURSOR_API_KEY` is intentionally absent. Credentials cross the cleared
/// child environment only when an operator explicitly names them in
/// `[execution.env].pass`; otherwise Cursor uses the login state under
/// `$HOME/.cursor`. [ORB-10945]
const REQUIRED_ENV_VARS: &[&str] = &["HOME", "PATH"];

pub(crate) struct CursorRuntime {
    command: String,
    cli: CursorCliTransport,
    runtime_key: &'static str,
    required_env_vars: &'static [&'static str],
}

pub(crate) struct CursorFactory;

impl CursorRuntime {
    pub(crate) fn new(
        command: String,
        model: Option<String>,
        runtime_key: &'static str,
        required_env_vars: &'static [&'static str],
    ) -> Self {
        Self {
            command,
            cli: CursorCliTransport::new(model),
            runtime_key,
            required_env_vars,
        }
    }
}

impl AgentRuntimeFactory for CursorFactory {
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
        Ok(ProviderOptions::Cursor)
    }

    fn build(&self, cfg: &AgentConfig) -> Result<Box<dyn AgentRuntime>, OrbitError> {
        match &cfg.provider_options {
            ProviderOptions::Cursor => Ok(Box::new(CursorRuntime::new(
                cfg.command.clone(),
                cfg.model.clone(),
                self.key(),
                self.required_env_vars(),
            ))),
            _ => Err(OrbitError::InvalidInput(format!(
                "provider options '{}' cannot build cursor runtime",
                cfg.provider_key
            ))),
        }
    }
}

impl AgentRuntime for CursorRuntime {
    fn invoke(
        &self,
        req: AgentRequest,
    ) -> Result<(AgentInvocationSpec, InvocationTrace), OrbitError> {
        Ok((
            crate::providers::build_invocation_spec(
                self.runtime_key,
                self.required_env_vars,
                self.command.clone(),
                self.cli.args(),
                self.cli.stdin(&req.envelope_json),
            ),
            InvocationTrace::default(),
        ))
    }

    fn model_name(&self) -> Option<&str> {
        self.cli.model_name()
    }
}
