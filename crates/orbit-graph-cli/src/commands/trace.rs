use clap::{Args, ValueEnum};
use orbit_graph::{DEFAULT_TRACE_DEPTH, RefConfidence};

use super::{CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub(crate) struct TraceCommand {
    command_name: String,
    #[arg(long, default_value_t = DEFAULT_TRACE_DEPTH)]
    depth: u8,
    #[arg(long, value_enum, default_value_t = ConfidenceArg::SameModule)]
    confidence: ConfidenceArg,
}

impl TraceCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let graph = context.open_graph()?;
        json_value(graph.trace(
            self.command_name.as_str(),
            self.depth,
            self.confidence.into_graph(),
        )?)
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
