use clap::{Args, ValueEnum};
use orbit_graph::OverviewFormat;
use orbit_graph_extract::Selector;

use super::{CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub struct OverviewCommand {
    /// Optional `dir:…` or `file:…` selector scoping the summary
    /// (default: whole worktree).
    scope: Option<String>,
    /// Output detail. `summary` returns counts plus the highest-symbol files;
    /// `full` lists every in-scope file with its symbols.
    #[arg(long, value_enum, default_value_t = FormatArg::Summary)]
    format: FormatArg,
}

impl OverviewCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let graph = context.open_graph()?;
        let scope = self
            .scope
            .as_deref()
            .map(str::parse::<Selector>)
            .transpose()?;
        json_value(graph.overview(scope.as_ref(), self.format.into_graph())?)
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[clap(rename_all = "snake_case")]
enum FormatArg {
    Summary,
    Full,
}

impl FormatArg {
    fn into_graph(self) -> OverviewFormat {
        match self {
            Self::Summary => OverviewFormat::Summary,
            Self::Full => OverviewFormat::Full,
        }
    }
}
