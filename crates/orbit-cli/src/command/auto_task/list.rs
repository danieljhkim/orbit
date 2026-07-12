use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::Value;

use crate::command::Execute;

use super::output::{definition_to_json, schedule_summary};

#[derive(Args)]
pub struct AutoTaskListArgs {
    /// Show only enabled (or, with `--disabled`, only disabled) definitions
    #[arg(long)]
    pub enabled: bool,
    /// Show only disabled definitions
    #[arg(long)]
    pub disabled: bool,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for AutoTaskListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let mut definitions = runtime.auto_task_list()?;
        if self.enabled {
            definitions.retain(|d| d.enabled);
        }
        if self.disabled {
            definitions.retain(|d| !d.enabled);
        }

        if self.json {
            let array = Value::Array(definitions.iter().map(definition_to_json).collect());
            return crate::output::json::print_pretty(&array);
        }

        for definition in &definitions {
            let state = if definition.enabled {
                "enabled"
            } else {
                "disabled"
            };
            println!(
                "{}\t{}\t{}\t{}",
                definition.name,
                state,
                schedule_summary(definition),
                definition.template.title
            );
        }
        Ok(())
    }
}
