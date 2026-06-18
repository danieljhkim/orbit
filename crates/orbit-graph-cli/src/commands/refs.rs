use clap::{Args, ValueEnum};
use orbit_graph::{RefConfidence, RefKind, RefOpts};
use orbit_graph_extract::Selector;

use super::{CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub struct RefsCommand {
    symbol: String,
    /// Minimum resolution confidence floor (default: same_module).
    ///
    /// The default keeps precise results. If the precise floor finds no
    /// references, the result auto-includes lower-confidence `fuzzy_name`
    /// (name-only) matches under a `fallback` field — so cross-crate callers
    /// routed through `pub use` re-exports are not silently hidden. Pass
    /// `--confidence fuzzy` to query those matches directly in `refs`.
    #[arg(long, value_enum, default_value_t = ConfidenceArg::SameModule)]
    confidence: ConfidenceArg,
    #[arg(long, value_enum)]
    kind: Option<RefKindArg>,
}

impl RefsCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let graph = context.open_graph()?;
        let selector = self.symbol.parse::<Selector>()?;
        let opts = RefOpts {
            confidence: self.confidence.into_graph(),
            kind: self.kind.map(RefKindArg::into_graph),
        };
        json_value(graph.refs(&selector, &opts)?)
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
