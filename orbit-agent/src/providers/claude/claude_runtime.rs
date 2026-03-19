use orbit_types::OrbitError;

use crate::providers::AgentProvider;
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
        Ok(AgentResponse {
            runtime_key: AgentProvider::Claude.key(),
            program: self.command.clone(),
            args: self.cli.args(&req.operation),
            stdin: self.cli.stdin(&req.envelope_json),
            stdout_schema_json: None,
            required_env_vars: AgentProvider::Claude.required_env_vars(),
        })
    }
}
