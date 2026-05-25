use clap::{Args, ValueEnum};
use orbit_graph::{GraphQueryKind, RefConfidence, RefKind, RefOpts};
use orbit_graph_extract::Selector;
use serde_json::json;

use super::{BackendArg, CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub(crate) struct RefsCommand {
    symbol: String,
    #[arg(long, value_enum, default_value_t = ConfidenceArg::SameModule)]
    confidence: ConfidenceArg,
    #[arg(long, value_enum)]
    kind: Option<RefKindArg>,
    #[arg(long, value_enum)]
    backend: Option<BackendArg>,
}

impl RefsCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let selector = self.symbol.parse::<Selector>()?;
        let opts = RefOpts {
            confidence: self.confidence.into_graph(),
            kind: self.kind.map(RefKindArg::into_graph),
        };
        let raw_selector = self.symbol.clone();
        let worktree = context.worktree_root.clone();
        context.route_query(
            self.backend,
            GraphQueryKind::Refs,
            move || {
                let graph =
                    orbit_graph::Graph::open(worktree.as_path(), orbit_graph::SyncPolicy::Manual)
                        .map_err(CliError::Graph)?;
                json_value(graph.refs(&selector, &opts)?)
            },
            || {
                context.run_legacy_tool(
                    "orbit.graph.refs",
                    json!({
                        "selector": raw_selector,
                    }),
                )
            },
        )
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[clap(rename_all = "snake_case")]
enum ConfidenceArg {
    Exact,
    #[value(alias = "import_resolved")]
    Import,
    SameModule,
    #[value(alias = "fuzzy_name")]
    Fuzzy,
}

impl ConfidenceArg {
    fn into_graph(self) -> RefConfidence {
        match self {
            Self::Exact => RefConfidence::Exact,
            Self::Import => RefConfidence::ImportResolved,
            Self::SameModule => RefConfidence::SameModule,
            Self::Fuzzy => RefConfidence::FuzzyName,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[clap(rename_all = "snake_case")]
enum RefKindArg {
    Call,
    Type,
    Use,
    TraitBound,
    Impl,
    Extends,
    Implements,
}

impl RefKindArg {
    fn into_graph(self) -> RefKind {
        match self {
            Self::Call => RefKind::Call,
            Self::Type => RefKind::Type,
            Self::Use => RefKind::Use,
            Self::TraitBound => RefKind::TraitBound,
            Self::Impl => RefKind::Impl,
            Self::Extends => RefKind::Extends,
            Self::Implements => RefKind::Implements,
        }
    }
}
