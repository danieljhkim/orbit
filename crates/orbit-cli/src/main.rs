// ORB-00004: legacy CLI binary surfaces still need a focused documentation pass.
#![allow(missing_docs)]
// ORB-00013: The CLI binary prints genuine user-facing command output.
#![allow(clippy::print_stderr, clippy::print_stdout)]
// ORB-00013: Unit tests use unwrap/expect for fixture setup; production call sites remain linted.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]
#![allow(
    rustdoc::broken_intra_doc_links,
    rustdoc::invalid_html_tags,
    rustdoc::private_intra_doc_links
)]

//! CLI entry point for Orbit: command parsing, dispatch, and output formatting.
//!
//! Parses command-line arguments with `clap`, initializes the [`OrbitRuntime`],
//! dispatches to the appropriate command handler, and formats results as JSON
//! or human-readable table output. Wraps every command in an audit middleware
//! that records success, failure, or policy-denial events.
//!
//! # Role
//! The outermost crate in the dependency graph. Depends on `orbit-core` and
//! `orbit-types`. All other crates are consumed transitively via `orbit-core`.
//! This binary is the `orbit` executable.
//!
//! # Key responsibilities
//! - Parse the top-level CLI surface and route subcommands to their handlers
//! - Bootstrap the runtime, including optional `--root` overrides for worktrees
//! - Emit machine-readable JSON or human-readable table output
//! - Wrap command execution in audit logging so human and agent actions are recorded
//!
//! # Dependency direction
//! orbit-core, orbit-types → `orbit-cli` (binary crate, no dependents)

mod audit_middleware;
mod command;
mod output;
mod parse;

use clap::Parser;
use orbit_core::{ActorIdentity, OrbitRuntime};

#[cfg(test)]
use crate::command::init::InitCommand;
use crate::command::operation::{CommandOperation, DispatchContext, RuntimeNeed};

fn main() {
    orbit_common::utility::logging::init_default_subscriber("warn");

    let cli = command::Cli::parse();
    let root_override = cli.root.clone();
    let CommandOperation {
        runtime_need,
        audit_meta,
        json_error_preference,
        suppress_errors,
        dispatch,
    } = cli.command.operation();

    if matches!(runtime_need, RuntimeNeed::Forbidden) {
        let result = dispatch(
            cli.command,
            DispatchContext::without_runtime(root_override.as_deref()),
        );
        finish_command(result, suppress_errors, json_error_preference);
        return;
    }

    let runtime = match OrbitRuntime::initialize_with_root_override(root_override.as_deref()) {
        Ok(runtime) => runtime,
        Err(err) => {
            if suppress_errors {
                return;
            }
            print_error(&err, json_error_preference);
            std::process::exit(1);
        }
    }
    // Direct CLI commands are human-driven by default. Tool-dispatch paths
    // reclassify themselves as agent-driven inside `execute_tool_command`.
    .with_actor(ActorIdentity::human("human"));

    let context = DispatchContext::with_runtime(&runtime, root_override.as_deref());
    let result = match audit_meta {
        Some(meta) => {
            let mut guard = audit_middleware::AuditGuard::new(&runtime, meta);
            let result = dispatch(cli.command, context);
            match &result {
                Ok(()) => guard.mark_success(),
                Err(orbit_core::OrbitError::PolicyDenied(msg)) => guard.mark_denied(msg),
                Err(err) => guard.mark_failure(err),
            }
            result
        }
        None => dispatch(cli.command, context),
    };

    finish_command(result, suppress_errors, json_error_preference);
}

fn finish_command(
    result: Result<(), orbit_core::OrbitError>,
    suppress_errors: bool,
    json_error_preference: Option<bool>,
) {
    if let Err(err) = result {
        if suppress_errors {
            return;
        }
        print_error(&err, json_error_preference);
        std::process::exit(1);
    }
}

fn print_error(error: &orbit_core::OrbitError, tool_run_json_output: Option<bool>) {
    if let Some(pretty) = tool_run_json_output {
        let payload = crate::output::json::error_payload(error);
        if crate::output::json::print_with_format(&payload, pretty).is_ok() {
            return;
        }
    }

    eprintln!("error: {error}");
}

#[cfg(test)]
mod tests;
