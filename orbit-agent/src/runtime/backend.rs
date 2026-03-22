use orbit_types::OrbitError;

use crate::providers::{ClaudeRuntime, CodexRuntime, MockAgentRuntime};
use crate::runtime::AgentRuntime;
use crate::types::{AgentRequest, AgentResponse};

#[allow(clippy::enum_variant_names)]
pub(crate) enum RuntimeBackend {
    CodexCli(CodexRuntime),
    ClaudeCli(ClaudeRuntime),
    MockAgentCli(MockAgentRuntime),
}

impl AgentRuntime for RuntimeBackend {
    fn invoke(&self, req: AgentRequest) -> Result<AgentResponse, OrbitError> {
        match self {
            RuntimeBackend::CodexCli(runtime) => runtime.invoke(req),
            RuntimeBackend::ClaudeCli(runtime) => runtime.invoke(req),
            RuntimeBackend::MockAgentCli(runtime) => runtime.invoke(req),
        }
    }

    fn model_name(&self) -> Option<&str> {
        match self {
            RuntimeBackend::CodexCli(runtime) => runtime.model_name(),
            RuntimeBackend::ClaudeCli(runtime) => runtime.model_name(),
            RuntimeBackend::MockAgentCli(runtime) => runtime.model_name(),
        }
    }
}
