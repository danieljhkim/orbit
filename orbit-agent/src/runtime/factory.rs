use orbit_types::OrbitError;

use crate::agent::AgentConfig;
use crate::providers::{AgentProvider, ClaudeRuntime, CodexRuntime, MockAgentRuntime};
use crate::runtime::RuntimeBackend;

pub(crate) fn resolve_runtime(cfg: &AgentConfig) -> Result<RuntimeBackend, OrbitError> {
    match AgentProvider::detect_from_cli(&cfg.command)? {
        AgentProvider::Codex => Ok(RuntimeBackend::CodexCli(CodexRuntime::new(
            cfg.command.clone(),
            cfg.model.clone(),
            cfg.codex_sandbox.clone(),
            cfg.codex_approval_policy.clone(),
        ))),
        AgentProvider::Claude => Ok(RuntimeBackend::ClaudeCli(ClaudeRuntime::new(
            cfg.command.clone(),
            cfg.model.clone(),
        ))),
        AgentProvider::MockAgent => Ok(RuntimeBackend::MockAgentCli(MockAgentRuntime::new(
            cfg.command.clone(),
        ))),
    }
}
