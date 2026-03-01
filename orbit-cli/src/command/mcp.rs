use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
pub struct McpCommand {
    #[command(subcommand)]
    pub command: McpSubcommand,
}

impl Execute for McpCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum McpSubcommand {
    /// Start MCP stdio server
    Start,
    /// Register Orbit MCP server in Codex and Claude configs
    Init(McpInitArgs),
}

impl Execute for McpSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            McpSubcommand::Start => orbit_mcp::serve_stdio(runtime),
            McpSubcommand::Init(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct McpInitArgs {
    /// Preview config updates without writing files
    #[arg(long)]
    pub dry_run: bool,
}

impl Execute for McpInitArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let result = runtime.init_mcp_configs(self.dry_run)?;
        print_summary("codex", &result.codex, self.dry_run);
        print_summary("claude", &result.claude, self.dry_run);
        Ok(())
    }
}

impl McpCommand {
    pub fn should_bypass_audit(&self) -> bool {
        matches!(self.command, McpSubcommand::Start)
    }
}

fn print_summary(
    name: &str,
    mutation: &orbit_core::command::mcp::McpConfigMutation,
    dry_run: bool,
) {
    let mode = if dry_run { "dry-run" } else { "applied" };
    let state = if mutation.changed {
        "updated"
    } else {
        "unchanged"
    };
    let existed = if mutation.existed { "existing" } else { "new" };
    println!(
        "{name}: {state} ({mode}, {existed}) -> {}",
        mutation.path.display()
    );
}
