//! Concrete agent provider implementations.
//!
//! Two families live here:
//!
//! - **CLI transports** (`claude`, `codex`, `gemini`, `grok`, `ollama`, `mock_agent`):
//!   translate an [`AgentRequest`] into a CLI command invocation and stdin
//!   envelope that the engine runs via `orbit-exec`.
//! - **HTTP transports** (`anthropic`, `openai_compat`, `gemini_http`): implement the sibling
//!   [`LoopTransport`](crate::loop_engine::LoopTransport) trait against a
//!   provider's HTTP API. Used by [`AgentLoop`](crate::loop_engine::AgentLoop)
//!   with explicit guardrails, allowlist enforcement, and audit wiring.
//!
//! The two families coexist: adding an HTTP transport does not remove the
//! existing CLI path, and the shared `AgentRuntime` trait is unchanged.

pub mod anthropic;
pub(crate) mod claude;
pub(crate) mod codex;
mod common;
pub(crate) mod gemini;
pub mod gemini_http;
pub(crate) mod grok;
pub(crate) mod mock_agent;
pub(crate) mod ollama;
pub mod openai_compat;

use crate::types::AgentInvocationSpec;

/// Builds the `AgentInvocationSpec` for a provider, combining CLI args and stdin.
/// All three runtimes share this same structure; only the provider differs.
pub(crate) fn build_invocation_spec(
    runtime_key: &'static str,
    required_env_vars: &'static [&'static str],
    command: String,
    args: Vec<String>,
    stdin: Vec<u8>,
) -> AgentInvocationSpec {
    AgentInvocationSpec {
        runtime_key,
        program: command,
        args,
        stdin,
        stdout_schema_json: None,
        required_env_vars,
    }
}
