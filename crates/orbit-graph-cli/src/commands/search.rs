use clap::{Args, ValueEnum};
use orbit_graph::{GraphQueryKind, SearchKind, SearchQuery};
use serde_json::json;

use super::{BackendArg, CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub(crate) struct SearchCommand {
    query: String,
    #[arg(long, value_enum)]
    kind: Option<SearchKindArg>,
    #[arg(long)]
    lang: Option<String>,
    #[arg(long)]
    limit: Option<usize>,
    #[arg(long, value_enum)]
    backend: Option<BackendArg>,
}

impl SearchCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let worktree = context.worktree_root.clone();
        let query = SearchQuery {
            query: self.query.clone(),
            kind: self.kind.map(SearchKindArg::into_graph),
            lang: self.lang.clone(),
            limit: self.limit,
        };
        let legacy_input = legacy_search_input(&query);
        context.route_query(
            self.backend,
            GraphQueryKind::Search,
            move || {
                let graph =
                    orbit_graph::Graph::open(worktree.as_path(), orbit_graph::SyncPolicy::Manual)
                        .map_err(CliError::Graph)?;
                json_value(graph.search(&query)?)
            },
            || context.run_legacy_tool("orbit.graph.search", legacy_input),
        )
    }
}

fn legacy_search_input(query: &SearchQuery) -> serde_json::Value {
    let mut input = json!({
        "query": query.query,
        "limit": query.limit,
    });
    if matches!(query.kind, Some(SearchKind::Symbol)) {
        input["type"] = json!("symbol");
    }
    input
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SearchKindArg {
    Symbol,
    String,
    Config,
}

impl SearchKindArg {
    fn into_graph(self) -> SearchKind {
        match self {
            Self::Symbol => SearchKind::Symbol,
            Self::String => SearchKind::String,
            Self::Config => SearchKind::Config,
        }
    }
}
