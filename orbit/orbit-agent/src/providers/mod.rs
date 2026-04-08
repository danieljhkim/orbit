//! Concrete agent provider implementations and CLI transport helpers.
//!
//! Each provider (`claude`, `codex`, `mock_agent`) translates an [`AgentRequest`]
//! into a CLI command invocation: a program path, argument list, and stdin bytes
//! that the engine will pass to `orbit-exec`. Providers are detected by inspecting
//! the `agent_cli` basename (e.g., `"claude"` → [`AgentProvider::Claude`]).
//!
//! The `common` module contains the shared `build_agent_response` helper used by
//! all three providers so that the CLI transport shape stays consistent regardless
//! of which AI backend is selected.

mod claude;
mod codex;
mod common;
mod gemini;
mod mock_agent;

use std::path::Path;

use orbit_types::OrbitError;

pub(crate) use claude::ClaudeRuntime;
pub(crate) use codex::CodexRuntime;
pub(crate) use gemini::GeminiRuntime;
pub(crate) use mock_agent::MockAgentRuntime;

use crate::types::AgentResponse;

/// Builds the `AgentResponse` for a provider, combining CLI args and stdin.
/// All three runtimes share this same structure; only the provider differs.
pub(crate) fn build_agent_response(
    provider: AgentProvider,
    command: String,
    args: Vec<String>,
    stdin: Vec<u8>,
) -> AgentResponse {
    AgentResponse {
        runtime_key: provider.key(),
        program: command,
        args,
        stdin,
        stdout_schema_json: None,
        required_env_vars: provider.required_env_vars(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentProvider {
    MockAgent,
    Codex,
    Claude,
    Gemini,
}

impl AgentProvider {
    pub(crate) fn key(self) -> &'static str {
        match self {
            AgentProvider::MockAgent => "mock-agent",
            AgentProvider::Codex => "codex",
            AgentProvider::Claude => "claude",
            AgentProvider::Gemini => "gemini",
        }
    }

    pub(crate) fn required_env_vars(self) -> &'static [&'static str] {
        match self {
            AgentProvider::MockAgent => &[],
            AgentProvider::Codex => &["HOME", "PATH"],
            AgentProvider::Claude => &["HOME", "PATH"],
            AgentProvider::Gemini => &["HOME", "PATH"],
        }
    }

    pub(crate) fn detect_from_cli(agent_cli: &str) -> Result<Self, OrbitError> {
        match provider_key(agent_cli).as_str() {
            "mock-agent" => Ok(AgentProvider::MockAgent),
            "codex" => Ok(AgentProvider::Codex),
            "claude" => Ok(AgentProvider::Claude),
            "gemini" => Ok(AgentProvider::Gemini),
            other => Err(OrbitError::UnsupportedAgentProvider(other.to_string())),
        }
    }
}

fn provider_key(agent_cli: &str) -> String {
    Path::new(agent_cli)
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_else(|| agent_cli.to_ascii_lowercase())
}
