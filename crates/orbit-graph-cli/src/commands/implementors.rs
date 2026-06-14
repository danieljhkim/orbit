use clap::Args;
use orbit_graph_extract::Selector;

use super::{CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub(crate) struct ImplementorsCommand {
    /// Trait selector (`symbol:<file>#<Trait>:trait` or `module:<path>`). The
    /// trait's trailing name segment is matched against impl sites.
    selector: String,
}

impl ImplementorsCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let graph = context.open_graph()?;
        let selector = self.selector.parse::<Selector>()?;
        json_value(graph.implementors(&selector)?)
    }
}
