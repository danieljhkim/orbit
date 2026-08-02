use clap::Args;
use orbit_core::{LearningUsageStat, OrbitRuntime};
use serde_json::Value;

use crate::command::{CommandOut, CommandOutput, Execute, Payload};
use crate::parse::parse_since;

#[derive(Args)]
pub struct LearningStatsArgs {
    /// Restrict the rollup to events since a duration or timestamp (e.g. "30d", RFC3339)
    #[arg(long)]
    pub since: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for LearningStatsArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let since = self.since.map(|s| parse_since(&s)).transpose()?;
        let stats = runtime.learning_usage_stats(since)?;

        if self.json {
            let array = Value::Array(stats.iter().map(usage_stat_to_json).collect());
            return Ok(Payload::document(array).into());
        }

        if stats.is_empty() {
            println!("no learning injection or show events recorded");
            return Ok(CommandOutput::Silent);
        }
        println!("ID\tINJECTED\tSHOWN\tSHOWN_RATIO\tLAST_INJECTED\tLAST_SHOWN");
        for stat in &stats {
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}",
                stat.learning_id,
                stat.injected_count,
                stat.shown_count,
                stat.shown_ratio()
                    .map(|ratio| format!("{ratio:.2}"))
                    .unwrap_or_else(|| "-".to_string()),
                stat.last_injected_at
                    .map(|ts| ts.to_rfc3339())
                    .unwrap_or_else(|| "-".to_string()),
                stat.last_shown_at
                    .map(|ts| ts.to_rfc3339())
                    .unwrap_or_else(|| "-".to_string()),
            );
        }
        Ok(CommandOutput::Silent)
    }
}

fn usage_stat_to_json(stat: &LearningUsageStat) -> Value {
    serde_json::json!({
        "learning_id": stat.learning_id,
        "injected_count": stat.injected_count,
        "shown_count": stat.shown_count,
        "shown_ratio": stat.shown_ratio(),
        "last_injected_at": stat.last_injected_at.map(|ts| ts.to_rfc3339()),
        "last_shown_at": stat.last_shown_at.map(|ts| ts.to_rfc3339()),
    })
}
