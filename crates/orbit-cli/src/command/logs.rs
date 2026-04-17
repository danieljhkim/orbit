use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;
use crate::command::job::summarize_error_message;

#[derive(Args)]
#[command(about = "View execution logs for a job run")]
pub struct LogsCommand {
    /// Run ID to inspect
    pub run_id: String,

    /// Show only a specific step by target_id
    #[arg(long)]
    pub step: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for LogsCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let run = runtime
            .show_job_run(&self.run_id)
            .map_err(|_| OrbitError::JobRunNotFound(self.run_id.clone()))?;

        let steps = if let Some(ref filter) = self.step {
            run.steps
                .iter()
                .filter(|s| s.target_id == *filter)
                .collect::<Vec<_>>()
        } else {
            run.steps.iter().collect()
        };

        if self.json {
            let values: Vec<serde_json::Value> = steps
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "step_index": s.step_index,
                        "target_id": s.target_id,
                        "target_type": s.target_type.to_string(),
                        "state": s.state.to_string(),
                        "started_at": s.started_at.map(|t| t.to_rfc3339()),
                        "finished_at": s.finished_at.map(|t| t.to_rfc3339()),
                        "duration_ms": s.duration_ms,
                        "exit_code": s.exit_code,
                        "error_code": s.error_code,
                        "error_message": s.error_message,
                    })
                })
                .collect();
            return crate::output::json::print_pretty(&serde_json::Value::Array(values));
        }

        use crate::output::color::{bold, dimmed, job_state_color};
        println!(
            "{} {}  {} {}  {} {}",
            bold("Run:"),
            run.run_id,
            bold("Job:"),
            run.job_id,
            bold("State:"),
            job_state_color(&run.state.to_string()),
        );
        if let Some(started) = run.started_at {
            println!("{} {}", bold("Started:"), dimmed(&started.to_rfc3339()));
        }
        if let Some(finished) = run.finished_at {
            println!("{} {}", bold("Finished:"), dimmed(&finished.to_rfc3339()));
        }
        if let Some(dur) = run.duration_ms {
            println!("{} {}ms", bold("Duration:"), dur);
        }
        println!();

        if steps.is_empty() {
            println!("No steps recorded.");
            return Ok(());
        }

        let mut table = crate::output::table::build_table(&[
            "#",
            "TARGET",
            "STATE",
            "DURATION",
            "ERROR CODE",
            "ERROR MESSAGE",
        ]);
        for s in &steps {
            use comfy_table::Cell;
            table.add_row(vec![
                Cell::new(s.step_index),
                Cell::new(&s.target_id),
                crate::output::color::job_state_color_cell(&s.state.to_string()),
                Cell::new(
                    s.duration_ms
                        .map(|d| format!("{d}ms"))
                        .unwrap_or_else(|| "-".to_string()),
                ),
                Cell::new(s.error_code.as_deref().unwrap_or("-")),
                Cell::new(summarize_error_message(s.error_message.as_deref())),
            ]);
        }
        println!("{table}");
        Ok(())
    }
}
