use clap::Args;
use orbit_graph::DEFAULT_IMPACT_DEPTH;
use orbit_graph_extract::Selector;

use super::{CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub(crate) struct ImpactCommand {
    selector: String,
    #[arg(long, default_value_t = DEFAULT_IMPACT_DEPTH)]
    depth: u8,
}

impl ImpactCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let graph = context.open_graph()?;
        let selector = self.selector.parse::<Selector>()?;
        json_value(graph.impact(&selector, self.depth)?)
    }
}
