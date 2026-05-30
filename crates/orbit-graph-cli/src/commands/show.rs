use clap::Args;
use orbit_graph::DEFAULT_SHOW_MAX_BYTES;
use orbit_graph_extract::Selector;

use super::{CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub(crate) struct ShowCommand {
    selector: String,
    #[arg(long, default_value_t = DEFAULT_SHOW_MAX_BYTES)]
    max_bytes: usize,
}

impl ShowCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let graph = context.open_graph()?;
        let selector = self.selector.parse::<Selector>()?;
        json_value(graph.show(&selector, self.max_bytes)?)
    }
}
