use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::output::definition_to_json;

#[derive(Args)]
pub struct AutoTaskShowArgs {
    /// Definition name
    pub name: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for AutoTaskShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let definition = runtime.auto_task_show(&self.name)?.ok_or_else(|| {
            OrbitError::InvalidInput(format!("no such auto-task '{}'", self.name))
        })?;

        if self.json {
            crate::output::json::print_pretty(&definition_to_json(&definition))
        } else {
            println!(
                "{} ({})",
                definition.name,
                if definition.enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            );
            if !definition.description.is_empty() {
                println!("  {}", definition.description);
            }
            println!(
                "  schedule: {}",
                super::output::schedule_summary(&definition)
            );
            println!("  dedupe: {:?}", definition.dedupe);
            println!("  template: {}", definition.template.title);
            Ok(())
        }
    }
}
