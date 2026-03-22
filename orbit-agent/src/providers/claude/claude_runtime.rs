use orbit_types::OrbitError;

use crate::providers::{AgentProvider, build_agent_response};
use crate::providers::claude::claude_cli::ClaudeCliTransport;
use crate::runtime::AgentRuntime;
use crate::types::{AgentRequest, AgentResponse};

pub(crate) struct ClaudeRuntime {
    command: String,
    cli: ClaudeCliTransport,
}

impl ClaudeRuntime {
    pub(crate) fn new(command: String, model: Option<String>) -> Self {
        Self {
            command,
            cli: ClaudeCliTransport::new(model),
        }
    }
}

impl AgentRuntime for ClaudeRuntime {
    fn invoke(&self, req: AgentRequest) -> Result<AgentResponse, OrbitError> {
        Ok(build_agent_response(
            AgentProvider::Claude,
            self.command.clone(),
            self.cli.args(),
            self.cli.stdin(&req.envelope_json),
        ))
    }

    fn model_name(&self) -> Option<&str> {
        self.cli.model_name()
    }
}
