use clap::{Args, ValueEnum};
use orbit_graph::{SearchKind, SearchQuery};

use super::{CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub struct SearchCommand {
    query: String,
    #[arg(long, value_enum)]
    kind: Option<SearchKindArg>,
    #[arg(long)]
    lang: Option<String>,
    #[arg(long)]
    limit: Option<usize>,
}

impl SearchCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let graph = context.open_graph()?;
        let query = SearchQuery {
            query: self.query.clone(),
            kind: self.kind.map(SearchKindArg::into_graph),
            lang: self.lang.clone(),
            limit: self.limit,
        };
        json_value(graph.search(&query)?)
    }
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
