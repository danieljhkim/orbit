use clap::{Args, ValueEnum};
use orbit_graph::{DEFAULT_IMPACT_DEPTH, RefConfidence};
use orbit_graph_extract::Selector;

use super::{CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub struct ImpactCommand {
    selector: String,
    #[arg(long, default_value_t = DEFAULT_IMPACT_DEPTH)]
    depth: u8,
    /// Minimum resolution confidence floor (default: same_module).
    ///
    /// The default traverses precise edges only. Cross-crate edges routed
    /// through `pub use` re-exports resolve at `fuzzy_name`; pass
    /// `--confidence fuzzy` to include them in the blast radius.
    #[arg(long, value_enum, default_value_t = ConfidenceArg::SameModule)]
    confidence: ConfidenceArg,
}

impl ImpactCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let graph = context.open_graph()?;
        let selector = self.selector.parse::<Selector>()?;
        json_value(graph.impact(&selector, self.depth, self.confidence.into_graph())?)
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
