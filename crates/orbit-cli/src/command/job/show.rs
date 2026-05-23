use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::support::{job_catalog_to_json_with_last_run, print_v2_step};

#[derive(Args)]
pub struct JobShowArgs {
    pub job_id: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for JobShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let job = runtime.show_job_catalog_entry(&self.job_id)?;
        if self.json {
            crate::output::json::print_pretty(&job_catalog_to_json_with_last_run(&job, None))
        } else {
            use crate::output::color::{bold, job_state_color};
            println!("{} {}", bold("Job ID:"), job.job_id.as_str());
            println!("{} {}", bold("Kind:"), job.kind());
            println!(
                "{} {}",
                bold("State:"),
                job_state_color(&job.state().to_string())
            );
            println!("{} {}", bold("Max Active Runs:"), job.max_active_runs());
            println!("{} {}", bold("Path:"), job.path.display());
            if let Some(default_input) = job.default_input() {
                let rendered = serde_json::to_string(default_input)
                    .unwrap_or_else(|_| "<invalid-json>".to_string());
                println!("{} {}", bold("Default Input:"), rendered);
            }
            println!("{} {}", bold("Steps:"), job.spec.steps.len());
            for (i, step) in job.spec.steps.iter().enumerate() {
                println!("  {}:", bold(&format!("Step {}", i + 1)));
                print_v2_step(step, 4);
            }
            Ok(())
        }
    }
}
