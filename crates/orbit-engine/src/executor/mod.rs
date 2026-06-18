//! Executor types and activity execution model for the Orbit engine.
//!
//! Defines the [`ActivityExecutor`] trait and its implementations:
//! - `direct_agent` — spawns an agent process directly from an ExecutorDef
//! - `external` — generic out-of-process executor speaking the External
//!   Executor Protocol v1 (shares the `direct_agent` transport)
//! - `automation` — runs built-in automation logic (task status updates, etc.)
//! - `cli_command` — executes an Orbit CLI sub-command as an activity step
//!
//! The `registry` module maps `spec_type` strings to the appropriate executor.

pub mod automation;
pub mod cli_command;
pub mod direct_agent;
pub mod external;
pub(crate) mod helpers;
pub mod registry;
pub mod traits;

pub(crate) use traits::ActivityExecutor;

#[cfg(test)]
mod tests;
