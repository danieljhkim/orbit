//! Agent provider abstraction for driving AI agents (Claude, Codex, Gemini, Ollama, mock) via CLI.
//!
//! Defines the [`AgentRuntime`] trait and provider implementations that translate
//! an [`AgentRequest`] (skill, input, tools) into a concrete CLI command and stdin
//! payload. The engine spawns the resulting command via `orbit-exec` and parses
//! the agent's JSON envelope response.
//!
//! # Role
//! Depends on `orbit-types` and is consumed by `orbit-engine`, which calls
//! `AgentRuntime::build_response` to obtain a runnable command descriptor and
//! then executes it through `orbit-exec`.
//!
//! # Key exports
//! - [`AgentRuntime`] trait — implement to add a new agent provider
//! - [`Agent`] / [`AgentConfig`] — high-level agent configuration
//! - [`AgentRequest`] / [`AgentInvocationSpec`] — request and CLI-command invocation types
//! - [`AgentOperation`] / [`AgentResponseStatus`] — operation kinds and status variants
//! - [`parse_and_validate_response`] — parses the agent's JSON envelope, usage, and tool traces
//!
//! # Dependency direction
//! `orbit-types` → `orbit-agent` → orbit-engine

mod agent;
mod providers;
mod runtime;
mod types;

pub use agent::{Agent, AgentConfig, ProviderOptions};
pub use orbit_types::{InvocationTrace, TokenUsage, ToolCallTrace};
pub use runtime::AgentRuntime;
pub use types::{AgentInvocationSpec, AgentOperation, AgentRequest, AgentResponseStatus};
pub use types::{is_timeout, parse_and_validate_response};
