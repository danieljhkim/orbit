use orbit_types::OrbitError;

use crate::providers::mock_agent::mock_agent_cli::MockAgentCliTransport;
use crate::providers::{AgentProvider, build_agent_response};
use crate::runtime::AgentRuntime;
use crate::types::{AgentRequest, AgentResponse};

pub(crate) struct MockAgentRuntime {
    command: String,
    cli: MockAgentCliTransport,
}

impl MockAgentRuntime {
    pub(crate) fn new(command: String) -> Self {
        Self {
            command,
            cli: MockAgentCliTransport,
        }
    }
}

impl AgentRuntime for MockAgentRuntime {
    fn invoke(&self, req: AgentRequest) -> Result<AgentResponse, OrbitError> {
        Ok(build_agent_response(
            AgentProvider::MockAgent,
            self.command.clone(),
            self.cli.args(&req.operation),
            self.cli.stdin(&req.envelope_json),
        ))
    }

    fn model_name(&self) -> Option<&str> {
        None
    }
}
