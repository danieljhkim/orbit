use clap::Args;
use orbit_graph::{DEFAULT_SHOW_MAX_BYTES, GraphQueryKind};
use orbit_graph_extract::Selector;
use serde_json::json;

use super::{BackendArg, CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub(crate) struct ShowCommand {
    selector: String,
    #[arg(long, default_value_t = DEFAULT_SHOW_MAX_BYTES)]
    max_bytes: usize,
    #[arg(long, value_enum)]
    backend: Option<BackendArg>,
}

impl ShowCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let selector = self.selector.parse::<Selector>()?;
        let raw_selector = self.selector.clone();
        let max_bytes = self.max_bytes;
        let worktree = context.worktree_root.clone();
        context.route_query(
            self.backend,
            GraphQueryKind::Show,
            move || {
                let graph =
                    orbit_graph::Graph::open(worktree.as_path(), orbit_graph::SyncPolicy::Manual)
                        .map_err(CliError::Graph)?;
                json_value(graph.show(&selector, max_bytes)?)
            },
            || {
                context.run_legacy_tool(
                    "orbit.graph.show",
                    json!({
                        "selector": raw_selector,
                    }),
                )
            },
        )
    }
}
