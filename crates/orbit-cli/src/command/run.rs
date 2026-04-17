use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "Execute a job by ID")]
pub struct RunCommand {
    /// Job ID to execute
    pub job_id: String,

    /// Input key=value pairs (repeatable)
    #[arg(long)]
    pub input: Vec<String>,

    /// Policy name to apply during execution
    #[arg(long)]
    pub policy: Option<String>,

    /// Stream agent stderr to terminal for debugging
    #[arg(long)]
    pub debug: bool,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for RunCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let input = build_input(&self.input)?;
        let result = runtime.run_job_now_with_input_debug(&self.job_id, input, self.debug)?;

        if self.json {
            crate::output::json::print_pretty(&json!({
                "job_id": result.job_id,
                "run_id": result.run_id,
                "state": result.state.to_string(),
                "attempt": result.attempt,
            }))
        } else {
            use crate::output::color::{bold, job_state_color};
            println!(
                "{} {} {} {} {} {}",
                bold("run_id:"),
                result.run_id,
                bold("state:"),
                job_state_color(&result.state.to_string()),
                bold("attempt:"),
                result.attempt,
            );
            Ok(())
        }
    }
}

fn build_input(pairs: &[String]) -> Result<Value, OrbitError> {
    let mut map = serde_json::Map::new();
    for pair in pairs {
        let (key, value) = pair.split_once('=').ok_or_else(|| {
            OrbitError::InvalidInput(format!("invalid --input \"{pair}\": expected key=value"))
        })?;
        let key = key.trim();
        if key.is_empty() {
            return Err(OrbitError::InvalidInput(format!(
                "invalid --input \"{pair}\": key must not be empty"
            )));
        }
        map.insert(key.to_string(), Value::String(value.to_string()));
    }
    Ok(Value::Object(map))
}
