use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::run;
use crate::command::{CommandOut, CommandOutput, Execute, Payload};

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
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        if self.task {
            return {
                show_task_artifacts(runtime, &self.id, self.json)?;
                Ok(CommandOutput::Silent)
            };
        }

        eprintln!("[deprecated] use \"orbit run show {}\"", self.id);
        if self.json {
            return match runtime.read_run_state(&self.id)? {
                Some(state) => Ok(Payload::document(
                    serde_json::to_value(&state).map_err(|e| OrbitError::Store(e.to_string()))?,
                )
                .into()),
                None => {
                    println!("No pipeline state found for run '{}'", self.id);
                    Ok(CommandOutput::Silent)
                }
            };
        }

        run::run_show_payload(runtime, Some(&self.id), None)
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
                    "media_type": a.media_type,
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
        println!(
            "--- {} ({}, {} bytes) ---",
            a.path,
            a.media_type,
            a.content.len()
        );
        if let Some(content) = a.text_content() {
            println!("{content}");
        } else {
            println!("[binary content omitted]");
        }
    }
    Ok(())
}
