use orbit_types::{InvocationTrace, OrbitError};

use crate::providers::AgentProvider;
use crate::providers::claude::claude_cli::ClaudeCliTransport;
use crate::runtime::AgentRuntime;
use crate::types::{AgentInvocationSpec, AgentRequest};

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
    fn invoke(
        &self,
        req: AgentRequest,
    ) -> Result<(AgentInvocationSpec, InvocationTrace), OrbitError> {
        Ok((
            crate::providers::build_invocation_spec(
                AgentProvider::Claude,
                self.command.clone(),
                self.cli.args(req.verbose),
                self.cli.stdin(&req.envelope_json),
            ),
            InvocationTrace::default(),
        ))
    }

    fn model_name(&self) -> Option<&str> {
        self.cli.model_name()
    }
}
