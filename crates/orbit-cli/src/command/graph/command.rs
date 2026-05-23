use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::build::GraphBuildArgs;
use super::history::GraphHistoryArgs;
use super::search::GraphSearchArgs;
use super::show::GraphShowArgs;
use super::update::GraphUpdateArgs;

#[derive(Args)]
#[command(about = "Build and query the knowledge graph")]
pub struct GraphCommand {
    #[command(subcommand)]
    pub subcommand: GraphSubcommand,
}

#[derive(Subcommand)]
pub enum GraphSubcommand {
    /// Build the knowledge graph from scratch
    Build(GraphBuildArgs),
    /// Incrementally update the knowledge graph
    Update(GraphUpdateArgs),
    /// Show a node and its context
    Show(GraphShowArgs),
    /// Search nodes by name or location
    Search(GraphSearchArgs),
    /// Compatibility stub for removed graph task attribution
    #[command(long_about = "Knowledge-graph task attribution has been removed. Use \
        `git log --grep '[T<task-id>]'` for local forward lookup, and use \
        `external_refs` for cross-engineer task references.")]
    History(GraphHistoryArgs),
}

impl Execute for GraphCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self.subcommand {
            GraphSubcommand::Build(args) => args.execute(runtime),
            GraphSubcommand::Update(args) => args.execute(runtime),
            GraphSubcommand::Show(args) => args.execute(runtime),
            GraphSubcommand::Search(args) => args.execute(runtime),
            GraphSubcommand::History(args) => args.execute(runtime),
        }
    }
}
