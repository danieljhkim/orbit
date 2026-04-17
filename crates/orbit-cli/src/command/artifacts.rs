use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "View artifacts for a job run or task")]
pub struct ArtifactsCommand {
    /// Run ID or task ID to inspect
    pub id: String,

    /// Treat the ID as a task ID instead of a run ID
    #[arg(long)]
    pub task: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for ArtifactsCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        if self.task {
            return show_task_artifacts(runtime, &self.id, self.json);
        }

        // Show run's pipeline state as the primary "artifact"
        match runtime.read_run_state(&self.id)? {
            Some(state) => {
                if self.json {
                    crate::output::json::print_pretty(
                        &serde_json::to_value(&state)
                            .map_err(|e| OrbitError::Store(e.to_string()))?,
                    )
                } else {
                    println!("Run:       {}", state.run_id);
                    println!("Job:       {}", state.job_id);
                    println!("Iteration: {}", state.iteration);
                    println!("Steps:     {}", state.step_outputs.len());
                    println!("Updated:   {}", state.updated_at.to_rfc3339());
                    if !state.step_outputs.is_empty() {
                        println!();
                        println!("Step outputs:");
                        for (key, _value) in &state.step_outputs {
                            println!("  - {key}");
                        }
                    }
                    Ok(())
                }
            }
            None => {
                println!("No pipeline state found for run '{}'", self.id);
                Ok(())
            }
        }
    }
}

fn show_task_artifacts(
    runtime: &OrbitRuntime,
    task_id: &str,
    as_json: bool,
) -> Result<(), OrbitError> {
    let artifacts = runtime.get_task_artifacts(task_id)?;

    if as_json {
        let values: Vec<serde_json::Value> = artifacts
            .iter()
            .map(|a| {
                serde_json::json!({
                    "path": a.path,
                    "size": a.content.len(),
                })
            })
            .collect();
        return crate::output::json::print_pretty(&serde_json::Value::Array(values));
    }

    if artifacts.is_empty() {
        println!("No artifacts found for task '{task_id}'.");
        return Ok(());
    }

    for a in &artifacts {
        println!("--- {} ({} bytes) ---", a.path, a.content.len());
        println!("{}", a.content);
    }
    Ok(())
}
