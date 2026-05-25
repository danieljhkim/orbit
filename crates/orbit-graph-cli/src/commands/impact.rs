use clap::{Args, ValueEnum};
use orbit_graph::{DEFAULT_IMPACT_DEPTH, GraphQueryKind, RefConfidence};
use orbit_graph_extract::Selector;

use super::{BackendArg, CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub(crate) struct ImpactCommand {
    selector: String,
    #[arg(long, default_value_t = DEFAULT_IMPACT_DEPTH)]
    depth: u8,
    #[arg(long, value_enum, default_value_t = ConfidenceArg::SameModule)]
    confidence: ConfidenceArg,
    #[arg(long, value_enum)]
    backend: Option<BackendArg>,
}

impl ImpactCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let selector = self.selector.parse::<Selector>()?;
        let depth = self.depth;
        let confidence = self.confidence.into_graph();
        let worktree = context.worktree_root.clone();
        context.route_query(
            self.backend,
            GraphQueryKind::Impact,
            move || {
                let graph =
                    orbit_graph::Graph::open(worktree.as_path(), orbit_graph::SyncPolicy::Manual)
                        .map_err(CliError::Graph)?;
                json_value(graph.impact(&selector, depth, confidence)?)
            },
            || context.legacy_unavailable("impact"),
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
