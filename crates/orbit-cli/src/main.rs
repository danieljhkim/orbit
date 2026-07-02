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

use crate::command::docs::{DocsCommand, DocsSubcommand};
use crate::command::friction::{FrictionCommand, FrictionSubcommand};
use crate::command::hook::{HookCommand, HookSubcommand};
use crate::command::learning::{LearningCommand, LearningSubcommand};
use crate::command::mcp::{McpCommand, McpSubcommand};
use crate::command::search::SearchCommand;
use crate::command::tool::{OutputFormat, ToolSubcommand};
use crate::command::web::{WebCommand, WebSubcommand};
use crate::command::workspace::{WorkspaceCommand, WorkspaceSubcommand};
use crate::command::{Commands, Execute, init::InitCommand};

fn main() {
    orbit_common::utility::logging::init_default_subscriber("warn");

    let cli = command::Cli::parse();
    let root_override = cli.root.clone();
    let tool_run_json_output = json_error_output_preference(&cli.command);
    let hook_pretooluse = is_hook_pretooluse_command(&cli.command);

    // Commands that run without a pre-existing runtime
    match cli.command {
        Commands::Init(cmd) => {
            if let Err(err) = execute_init_command(cmd, root_override.as_deref()) {
                print_error(&err, tool_run_json_output);
                std::process::exit(1);
            }
            return;
        }
        Commands::Workspace(WorkspaceCommand {
            command: WorkspaceSubcommand::Init(args),
        }) => {
            if let Err(err) = args.execute_without_runtime(root_override.as_deref()) {
                print_error(&err, tool_run_json_output);
                std::process::exit(1);
            }
            return;
        }
        Commands::Mcp(McpCommand {
            command: McpSubcommand::Init(args),
        }) => {
            if let Err(err) = args.execute_without_runtime(root_override.as_deref()) {
                print_error(&err, tool_run_json_output);
                std::process::exit(1);
            }
            return;
        }
        Commands::Mcp(McpCommand {
            command: McpSubcommand::Remove(args),
        }) => {
            if let Err(err) = args.execute_without_runtime(root_override.as_deref()) {
                print_error(&err, tool_run_json_output);
                std::process::exit(1);
            }
            return;
        }
        Commands::Mcp(McpCommand {
            command: McpSubcommand::Serve(args),
        }) => {
            if let Err(err) = args.execute_without_runtime(root_override.as_deref()) {
                print_error(&err, tool_run_json_output);
                std::process::exit(1);
            }
            return;
        }
        Commands::Learning(LearningCommand {
            command: LearningSubcommand::MigrateLayout(args),
        }) => {
            if let Err(err) = args.execute_without_runtime(root_override.as_deref()) {
                print_error(&err, tool_run_json_output);
                std::process::exit(1);
            }
            return;
        }
        // `orbit web` resolves its own workspace(s): `serve` can serve globally
        // from any directory, and `connect` is a client-side SSH tunnel whose
        // workspace lives on the remote. Both must dispatch before the eager
        // workspace initialization below (which fails outside a workspace).
        Commands::Web(WebCommand { command }) => {
            let result = match command {
                WebSubcommand::Serve(args) => {
                    orbit_dashboard::serve_from_env(args, root_override.as_deref())
                }
                WebSubcommand::Connect(args) => orbit_dashboard::connect(args),
            };
            if let Err(err) = result {
                print_error(&err, tool_run_json_output);
                std::process::exit(1);
            }
            return;
        }
        _ => {}
    }

    let runtime = match OrbitRuntime::initialize_with_root_override(root_override.as_deref()) {
        Ok(runtime) => runtime,
        Err(err) => {
            if hook_pretooluse {
                return;
            }
            print_error(&err, tool_run_json_output);
            std::process::exit(1);
        }
    }
    // Direct CLI commands are human-driven by default. Tool-dispatch paths
    // reclassify themselves as agent-driven inside `execute_tool_command`.
    .with_actor(ActorIdentity::human("human"));

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
        if hook_pretooluse {
            return;
        }
        print_error(&err, tool_run_json_output);
        std::process::exit(1);
    }
}

fn execute_init_command(
    cmd: InitCommand,
    root_override: Option<&std::path::Path>,
) -> Result<(), orbit_core::OrbitError> {
    cmd.execute_without_runtime(root_override)
}

fn json_error_output_preference(command: &Commands) -> Option<bool> {
    match command {
        Commands::Tool(command) => match &command.command {
            ToolSubcommand::Run(args) if matches!(args.output, OutputFormat::Json) => {
                Some(args.pretty)
            }
            _ => None,
        },
        Commands::Docs(DocsCommand { command }) => match command {
            DocsSubcommand::List(args) => args.json.then_some(true),
            DocsSubcommand::Show(args) => args.json.then_some(true),
            DocsSubcommand::Add(args) => args.json.then_some(true),
            DocsSubcommand::Index(args) => args.json.then_some(true),
            DocsSubcommand::Migrate(args) => args.json.then_some(true),
        },
        Commands::Friction(FrictionCommand { command }) => match command {
            FrictionSubcommand::Add(args) => args.json.then_some(true),
            FrictionSubcommand::List(args) => args.json.then_some(true),
            FrictionSubcommand::Show(args) => args.json.then_some(true),
            FrictionSubcommand::Stats(args) => args.json.then_some(true),
            FrictionSubcommand::Tags(args) => args.json.then_some(true),
            FrictionSubcommand::Update(args) => args.json.then_some(true),
            FrictionSubcommand::Resolve(args) => args.json.then_some(true),
        },
        Commands::Search(SearchCommand { json: true, .. }) => Some(true),
        _ => None,
    }
}

fn is_hook_pretooluse_command(command: &Commands) -> bool {
    matches!(
        command,
        Commands::Hook(HookCommand {
            command: HookSubcommand::Pretooluse(_)
        })
    )
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
