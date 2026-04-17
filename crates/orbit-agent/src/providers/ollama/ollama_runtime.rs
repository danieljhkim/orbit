use orbit_types::{InvocationTrace, OrbitError};

use crate::providers::ollama::ollama_cli::OllamaCliTransport;
use crate::providers::{AgentProvider, build_agent_response};
use crate::runtime::AgentRuntime;
use crate::types::{AgentRequest, AgentResponse};

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
    fn invoke(&self, req: AgentRequest) -> Result<(AgentResponse, InvocationTrace), OrbitError> {
        Ok((
            build_agent_response(
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
