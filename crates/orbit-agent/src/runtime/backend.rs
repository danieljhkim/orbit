use orbit_types::{InvocationTrace, OrbitError};

use crate::providers::{
    ClaudeRuntime, CodexRuntime, GeminiRuntime, MockAgentRuntime, OllamaRuntime,
};
use crate::runtime::AgentRuntime;
use crate::types::{AgentInvocationSpec, AgentRequest};

#[allow(clippy::enum_variant_names)]
pub(crate) enum RuntimeBackend {
    CodexCli(CodexRuntime),
    ClaudeCli(ClaudeRuntime),
    GeminiCli(GeminiRuntime),
    OllamaCli(OllamaRuntime),
    MockAgentCli(MockAgentRuntime),
}

impl AgentRuntime for RuntimeBackend {
    fn invoke(
        &self,
        req: AgentRequest,
    ) -> Result<(AgentInvocationSpec, InvocationTrace), OrbitError> {
        match self {
            RuntimeBackend::CodexCli(runtime) => runtime.invoke(req),
            RuntimeBackend::ClaudeCli(runtime) => runtime.invoke(req),
            RuntimeBackend::GeminiCli(runtime) => runtime.invoke(req),
            RuntimeBackend::OllamaCli(runtime) => runtime.invoke(req),
            RuntimeBackend::MockAgentCli(runtime) => runtime.invoke(req),
        }
    }

    fn model_name(&self) -> Option<&str> {
        match self {
            RuntimeBackend::CodexCli(runtime) => runtime.model_name(),
            RuntimeBackend::ClaudeCli(runtime) => runtime.model_name(),
            RuntimeBackend::GeminiCli(runtime) => runtime.model_name(),
            RuntimeBackend::OllamaCli(runtime) => runtime.model_name(),
            RuntimeBackend::MockAgentCli(runtime) => runtime.model_name(),
        }
    }
}
