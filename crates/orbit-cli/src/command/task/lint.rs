use clap::Args;
use orbit_core::command::task::TaskLintSeverity;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
pub struct TaskLintArgs {
    /// Task ID
    pub id: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskLintArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let report = runtime.lint_task(&self.id)?;

        if self.json {
            let value = serde_json::to_value(&report).map_err(|e| OrbitError::Io(e.to_string()))?;
            return crate::output::json::print_pretty(&value);
        }

        if report.findings.is_empty() {
            println!(
                "No lint findings for '{}' ({} ms).",
                report.task_id, report.duration_ms
            );
            return Ok(());
        }

        println!(
            "{} finding(s) for '{}' ({} ms):",
            report.finding_count, report.task_id, report.duration_ms
        );
        for finding in report.findings {
            let severity = match finding.severity {
                TaskLintSeverity::Error => "error",
                TaskLintSeverity::Warning => "warning",
            };
            println!("[{severity}] {}: {}", finding.check, finding.message);
            println!("  fix: {}", finding.fix_it);
        }
        Ok(())
    }
}
