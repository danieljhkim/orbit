use crate::providers::AgentProvider;
use crate::providers::codex::codex_cli::CodexCliTransport;
use crate::runtime::AgentRuntime;
use crate::types::{AgentInvocationSpec, AgentRequest};
use orbit_types::{InvocationTrace, OrbitError};

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
    fn invoke(
        &self,
        req: AgentRequest,
    ) -> Result<(AgentInvocationSpec, InvocationTrace), OrbitError> {
        // Note: stdout_schema_json is intentionally None — Codex rejects Orbit's
        // generic envelope schema because open-ended object branches must be closed
        // with additionalProperties=false. Orbit validates the returned envelope
        // after execution instead.
        Ok((
            crate::providers::build_invocation_spec(
                AgentProvider::Codex,
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
