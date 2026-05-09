#![deny(clippy::print_stderr, clippy::print_stdout)]
//! Process spawning, sandboxing, and timeout handling for Orbit tool execution.
//!
//! Provides the low-level primitives for launching child processes with
//! controlled environments, optional sandboxing, and configurable timeouts.
//! Results are captured and returned as [`ExecutionResult`] values.
//!
//! # Role
//! Sits directly above `orbit-types` and is consumed by `orbit-tools`, which
//! builds the builtin `proc.spawn` tool and other shell-invoking tools on top
//! of these primitives.
//!
//! # Key exports
//! - [`run_process`] — primary entry point for spawning a subprocess
//! - [`ExecRequest`] — builder-style description of the process to run
//! - [`ExecutionResult`] — captured stdout/stderr, exit code, and duration
//! - [`Sandbox`] / [`NoSandbox`] — sandbox strategy trait and no-op default
//! - [`EnvironmentMode`], [`StdinMode`] — environment and stdin control
//!
//! # Dependency direction
//! `orbit-types` → `orbit-exec` → orbit-tools

pub mod macos_sandbox;
pub mod process;
pub mod result;
pub mod runner;
pub mod sandbox;
mod supervision;

pub use macos_sandbox::{
    MacosSandboxSpawnRequest, claude_state_dir_from_env, compile_macos_sandbox_profile,
    sandbox_exec_available, sandbox_exec_path, sandbox_exec_program_for_audit,
    sandbox_exec_unavailable_message, spawn_under_macos_sandbox,
};
pub use result::ExecutionResult;
pub use runner::{EnvironmentMode, ExecRequest, StdinMode, run_process};
pub use sandbox::{NoSandbox, Sandbox};
