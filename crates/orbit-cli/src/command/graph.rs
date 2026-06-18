use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};
use orbit_graph_cli::Command as GraphSubcommand;

use super::Execute;

/// Query and sync the Orbit code graph (orbit-graph v2).
///
/// Thin wrapper over the `orbit-graph-cli` library: the subcommands are
/// worktree-scoped (the graph DB is discovered from the current git worktree)
/// and emit JSON, so this command dispatches and prints the payload rather than
/// going through `OrbitRuntime`. The standalone `orbit-graph-cli` binary shares
/// the same command layer.
#[derive(Args)]
pub struct GraphCommand {
    #[command(subcommand)]
    pub command: GraphSubcommand,
}

impl Execute for GraphCommand {
    fn execute(self, _runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let value = self
            .command
            .run()
            .map_err(|error| OrbitError::Execution(error.to_string()))?;
        crate::output::json::print(&value)
    }
}
