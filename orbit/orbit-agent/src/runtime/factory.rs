use orbit_types::OrbitError;

use crate::agent::{AgentConfig, ProviderOptions};
use crate::providers::{ClaudeRuntime, CodexRuntime, GeminiRuntime, MockAgentRuntime};
use crate::runtime::RuntimeBackend;

pub(crate) fn resolve_runtime(cfg: &AgentConfig) -> Result<RuntimeBackend, OrbitError> {
    match &cfg.provider_options {
        ProviderOptions::Codex {
            sandbox,
            approval_policy,
            writable_dirs,
        } => Ok(RuntimeBackend::CodexCli(CodexRuntime::new(
            cfg.command.clone(),
            cfg.model.clone(),
            sandbox.clone(),
            approval_policy.clone(),
            writable_dirs.clone(),
        ))),
        ProviderOptions::Claude => Ok(RuntimeBackend::ClaudeCli(ClaudeRuntime::new(
            cfg.command.clone(),
            cfg.model.clone(),
        ))),
        ProviderOptions::Gemini => Ok(RuntimeBackend::GeminiCli(GeminiRuntime::new(
            cfg.command.clone(),
            cfg.model.clone(),
        ))),
        ProviderOptions::Mock => Ok(RuntimeBackend::MockAgentCli(MockAgentRuntime::new(
            cfg.command.clone(),
        ))),
    }
}
