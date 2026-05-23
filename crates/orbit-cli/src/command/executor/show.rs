use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::support::executor_def_json;

#[derive(Args)]
pub struct ExecutorShowArgs {
    pub name: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for ExecutorShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let def = runtime.get_executor_def(&self.name)?.ok_or_else(|| {
            OrbitError::InvalidInput(format!("executor not found: {}", self.name))
        })?;
        if self.json {
            crate::output::json::print_pretty(&executor_def_json(&def))
        } else {
            println!("Name:      {}", def.name);
            println!("Type:      {}", def.executor_type);
            if let Some(ref cmd) = def.command {
                println!("Command:   {cmd}");
            }
            if !def.args.is_empty() {
                println!("Args:      {}", def.args.join(" "));
            }
            if let Some(ref fmt) = def.stdout_format {
                println!("Stdout:    {fmt}");
            }
            if let Some(timeout) = def.timeout_seconds {
                println!("Timeout:   {timeout}s");
            }
            if !def.env.is_empty() {
                println!("Env:");
                for (k, v) in &def.env {
                    println!("  {k}={v}");
                }
            }
            println!("Created:   {}", def.created_at);
            println!("Updated:   {}", def.updated_at);
            Ok(())
        }
    }
}
