use clap::Args;
use orbit_graph_extract::Selector;

use super::{CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub struct DepsCommand {
    /// File or directory selector (`file:…` or `dir:…`) whose outbound module/
    /// import edges to list. Reports source-level imports, not Cargo crate edges.
    selector: String,
}

impl DepsCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let graph = context.open_graph()?;
        let selector = self.selector.parse::<Selector>()?;
        json_value(graph.deps(&selector)?)
    }
}
