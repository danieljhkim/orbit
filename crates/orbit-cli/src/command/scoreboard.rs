use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "Generate read-only summaries from Orbit scoreboards")]
pub struct ScoreboardCommand {
    #[command(subcommand)]
    pub command: ScoreboardSubcommand,
}

impl Execute for ScoreboardCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum ScoreboardSubcommand {
    /// Generate `.orbit/state/scoreboard/summary.json` on demand
    Summary(ScoreboardSummaryArgs),
}

impl Execute for ScoreboardSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            ScoreboardSubcommand::Summary(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct ScoreboardSummaryArgs {
    /// Output generated summary JSON to stdout
    #[arg(long)]
    pub json: bool,
}

impl Execute for ScoreboardSummaryArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let summary = runtime.generate_scoreboard_summary()?;

        if self.json {
            return crate::output::json::print_pretty(
                &serde_json::to_value(&summary).map_err(|e| OrbitError::Store(e.to_string()))?,
            );
        }

        println!(
            "wrote {} ({} agent entries)",
            runtime.scoreboard_summary_path().display(),
            summary.agents.len()
        );
        Ok(())
    }
}
