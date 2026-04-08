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
use orbit_core::OrbitRuntime;

use crate::command::workspace::{WorkspaceCommand, WorkspaceSubcommand};
use crate::command::{Commands, Execute, init::InitCommand};

fn main() {
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .try_init();

    let cli = command::Cli::parse();
    let root_override = cli.root.clone();

    // Commands that run without a pre-existing runtime
    match cli.command {
        Commands::Init(cmd) => {
            if let Err(err) = execute_init_command(cmd, root_override.as_deref()) {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
            return;
        }
        Commands::Workspace(WorkspaceCommand {
            command: WorkspaceSubcommand::Init(args),
        }) => {
            if let Err(err) = args.execute_without_runtime() {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
            return;
        }
        _ => {}
    }

    let runtime = match OrbitRuntime::initialize_with_root_override(root_override.as_deref()) {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("failed to initialize runtime: {err}");
            std::process::exit(1);
        }
    };

    let result = match cli.command {
        Commands::Audit(cmd) => cmd.execute(&runtime),
        other => {
            let meta = audit_middleware::extract_command_meta(&other);
            let mut guard = audit_middleware::AuditGuard::new(&runtime, meta);
            let result = other.execute(&runtime);
            match &result {
                Ok(()) => guard.mark_success(),
                Err(orbit_core::OrbitError::PolicyDenied(msg)) => guard.mark_denied(msg),
                Err(err) => guard.mark_failure(err),
            }
            result
        }
    };

    if let Err(err) = result {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn execute_init_command(
    cmd: InitCommand,
    root_override: Option<&std::path::Path>,
) -> Result<(), orbit_core::OrbitError> {
    cmd.execute_without_runtime(root_override)
}
