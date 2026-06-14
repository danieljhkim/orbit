use clap::{Args, ValueEnum};
use orbit_graph::{DEFAULT_TRACE_DEPTH, RefConfidence};

use super::{CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub(crate) struct TraceCommand {
    command_name: String,
    #[arg(long, default_value_t = DEFAULT_TRACE_DEPTH)]
    depth: u8,
    /// Minimum resolution confidence floor (default: same_module).
    ///
    /// The default follows precise edges only. Cross-crate edges routed
    /// through `pub use` re-exports resolve at `fuzzy_name`; pass
    /// `--confidence fuzzy` to follow them while tracing.
    #[arg(long, value_enum, default_value_t = ConfidenceArg::SameModule)]
    confidence: ConfidenceArg,
}

impl TraceCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let graph = context.open_graph()?;
        json_value(graph.trace(
            normalize_command_selector(self.command_name.as_str()),
            self.depth,
            self.confidence.into_graph(),
        )?)
    }
}

fn normalize_command_selector(command: &str) -> &str {
    command
        .trim()
        .strip_prefix("command:")
        .map(str::trim)
        .unwrap_or_else(|| command.trim())
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
