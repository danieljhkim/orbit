use orbit_types::OrbitError;

use crate::providers::gemini::gemini_cli::GeminiCliTransport;
use crate::providers::{AgentProvider, build_agent_response};
use crate::runtime::AgentRuntime;
use crate::types::{AgentRequest, AgentResponse};

pub(crate) struct GeminiRuntime {
    command: String,
    cli: GeminiCliTransport,
}

impl GeminiRuntime {
    pub(crate) fn new(command: String, model: Option<String>) -> Self {
        Self {
            command,
            cli: GeminiCliTransport::new(model),
        }
    }
}

impl AgentRuntime for GeminiRuntime {
    fn invoke(&self, req: AgentRequest) -> Result<AgentResponse, OrbitError> {
        Ok(build_agent_response(
            AgentProvider::Gemini,
            self.command.clone(),
            self.cli.args(req.verbose),
            self.cli.stdin(&req.envelope_json),
        ))
    }

    fn model_name(&self) -> Option<&str> {
        self.cli.model_name()
    }
}
