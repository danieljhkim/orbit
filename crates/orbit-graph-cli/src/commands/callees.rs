use clap::Args;
use orbit_graph_extract::Selector;

use super::{CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub(crate) struct CalleesCommand {
    symbol: String,
}

impl CalleesCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let graph = context.open_graph()?;
        let selector = self.symbol.parse::<Selector>()?;
        json_value(serde_json::json!({
            "callees": graph.callees(&selector)?,
        }))
    }
}
