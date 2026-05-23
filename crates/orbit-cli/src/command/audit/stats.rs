use clap::Args;
use orbit_core::{AuditStats, OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;
use crate::parse::parse_since;

#[derive(Args)]
pub struct AuditStatsArgs {
    /// Stats since duration or timestamp
    #[arg(long)]
    pub since: Option<String>,
    /// Filter by tool name
    #[arg(long)]
    pub tool: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for AuditStatsArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let since = self.since.map(|s| parse_since(&s)).transpose()?;
        let stats = runtime.audit_event_stats(since, self.tool)?;

        if self.json {
            crate::output::json::print_pretty(&stats_to_json(&stats))
        } else {
            println!("Total:             {}", stats.total);
            println!("Success:           {}", stats.success_count);
            println!("Failure:           {}", stats.failure_count);
            println!("Denied:            {}", stats.denied_count);
            println!("Avg duration (ms): {:.1}", stats.avg_duration_ms);
            println!("P95 duration (ms): {}", stats.p95_duration_ms);
            println!("Max duration (ms): {}", stats.max_duration_ms);
            Ok(())
        }
    }
}

fn stats_to_json(stats: &AuditStats) -> Value {
    json!({
        "total": stats.total,
        "success_count": stats.success_count,
        "failure_count": stats.failure_count,
        "denied_count": stats.denied_count,
        "avg_duration_ms": stats.avg_duration_ms,
        "p95_duration_ms": stats.p95_duration_ms,
        "max_duration_ms": stats.max_duration_ms,
    })
}
