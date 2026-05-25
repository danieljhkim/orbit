use clap::Args;
use orbit_graph::DEFAULT_TRACE_DEPTH;

use super::{CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub(crate) struct TraceCommand {
    command_name: String,
    #[arg(long, default_value_t = DEFAULT_TRACE_DEPTH)]
    depth: u8,
}

impl TraceCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let graph = context.open_graph()?;
        json_value(graph.trace(self.command_name.as_str(), self.depth)?)
    }
}
