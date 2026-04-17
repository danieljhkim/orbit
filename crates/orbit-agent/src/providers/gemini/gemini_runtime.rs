use orbit_types::{InvocationTrace, OrbitError};

use crate::providers::AgentProvider;
use crate::providers::gemini::gemini_cli::GeminiCliTransport;
use crate::runtime::AgentRuntime;
use crate::types::{AgentInvocationSpec, AgentRequest};

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
    fn invoke(
        &self,
        req: AgentRequest,
    ) -> Result<(AgentInvocationSpec, InvocationTrace), OrbitError> {
        Ok((
            crate::providers::build_invocation_spec(
                AgentProvider::Gemini,
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
