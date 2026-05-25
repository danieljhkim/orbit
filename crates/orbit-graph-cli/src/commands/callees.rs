use clap::Args;
use orbit_graph::GraphQueryKind;
use orbit_graph_extract::Selector;

use super::{BackendArg, CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub(crate) struct CalleesCommand {
    symbol: String,
    #[arg(long, value_enum)]
    backend: Option<BackendArg>,
}

impl CalleesCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let selector = self.symbol.parse::<Selector>()?;
        let worktree = context.worktree_root.clone();
        context.route_query(
            self.backend,
            GraphQueryKind::Callees,
            move || {
                let graph =
                    orbit_graph::Graph::open(worktree.as_path(), orbit_graph::SyncPolicy::Manual)
                        .map_err(CliError::Graph)?;
                json_value(serde_json::json!({
                    "callees": graph.callees(&selector)?,
                }))
            },
            || context.legacy_unavailable("callees"),
        )
    }
}
