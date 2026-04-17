use orbit_types::{InvocationTrace, OrbitError};

use crate::providers::AgentProvider;
use crate::providers::ollama::ollama_cli::OllamaCliTransport;
use crate::runtime::AgentRuntime;
use crate::types::{AgentInvocationSpec, AgentRequest};

pub(crate) struct OllamaRuntime {
    command: String,
    cli: OllamaCliTransport,
}

impl OllamaRuntime {
    pub(crate) fn new(command: String, model: Option<String>) -> Result<Self, OrbitError> {
        Ok(Self {
            command,
            cli: OllamaCliTransport::new(model)?,
        })
    }
}

impl AgentRuntime for OllamaRuntime {
    fn invoke(
        &self,
        req: AgentRequest,
    ) -> Result<(AgentInvocationSpec, InvocationTrace), OrbitError> {
        Ok((
            crate::providers::build_invocation_spec(
                AgentProvider::Ollama,
                self.command.clone(),
                self.cli.args(req.verbose),
                self.cli.stdin(&req.envelope_json),
            ),
            InvocationTrace::default(),
        ))
    }

    fn model_name(&self) -> Option<&str> {
        Some(self.cli.model_name())
    }
}
