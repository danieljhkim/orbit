//! Executor types and activity execution model for the Orbit engine.
//!
//! Defines the [`ActivityExecutor`] trait and its implementations:
//! - `agent` — invokes an AI agent (Claude, Codex) via the `orbit-agent` provider
//! - `automation` — runs built-in automation logic (task status updates, etc.)
//! - `cli_command` — executes an Orbit CLI sub-command as an activity step
//!
//! The `registry` module maps `spec_type` strings (e.g., `"agent_invoke"`) to the
//! appropriate executor. Each executor receives an [`ExecutionContext`] and returns
//! an [`AttemptOutcome`], which the job runner uses to decide on retry or advance.

pub mod agent;
pub mod automation;
pub mod cli_command;
pub mod registry;
pub mod traits;

pub(crate) use registry::builtin_activity_executor_registry;
pub(crate) use traits::ActivityExecutor;
