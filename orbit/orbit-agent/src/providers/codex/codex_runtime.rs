use crate::providers::codex::codex_cli::CodexCliTransport;
use crate::providers::{AgentProvider, build_agent_response};
use crate::runtime::AgentRuntime;
use crate::types::{AgentRequest, AgentResponse};
use orbit_types::OrbitError;

pub(crate) struct CodexRuntime {
    command: String,
    cli: CodexCliTransport,
}

impl CodexRuntime {
    pub(crate) fn new(
        command: String,
        model: Option<String>,
        sandbox: String,
        approval_policy: Option<String>,
        writable_dirs: Vec<String>,
    ) -> Self {
        Self {
            command,
            cli: CodexCliTransport::new(model, sandbox, approval_policy, writable_dirs),
        }
    }
}

impl AgentRuntime for CodexRuntime {
    fn invoke(&self, req: AgentRequest) -> Result<AgentResponse, OrbitError> {
        // Note: stdout_schema_json is intentionally None — Codex rejects Orbit's
        // generic envelope schema because open-ended object branches must be closed
        // with additionalProperties=false. Orbit validates the returned envelope
        // after execution instead.
        Ok(build_agent_response(
            AgentProvider::Codex,
            self.command.clone(),
            self.cli.args(),
            self.cli.stdin(&req.envelope_json),
        ))
    }

    fn model_name(&self) -> Option<&str> {
        self.cli.model_name()
    }
}
