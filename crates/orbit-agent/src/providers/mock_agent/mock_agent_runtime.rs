use orbit_types::{InvocationTrace, OrbitError};

use crate::providers::AgentProvider;
use crate::providers::mock_agent::mock_agent_cli::MockAgentCliTransport;
use crate::runtime::AgentRuntime;
use crate::types::{AgentInvocationSpec, AgentRequest};

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
    fn invoke(
        &self,
        req: AgentRequest,
    ) -> Result<(AgentInvocationSpec, InvocationTrace), OrbitError> {
        Ok((
            crate::providers::build_invocation_spec(
                AgentProvider::MockAgent,
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
