use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::Value;

use crate::command::Execute;

use super::support::executor_def_json;

#[derive(Args)]
pub struct ExecutorListArgs {
    #[arg(long)]
    pub json: bool,
}

impl Execute for ExecutorListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let defs = runtime.list_executor_defs()?;
        if self.json {
            let values: Vec<Value> = defs.iter().map(executor_def_json).collect();
            crate::output::json::print_pretty(&Value::Array(values))
        } else {
            let mut table =
                crate::output::table::build_table(&["NAME", "TYPE", "COMMAND", "TIMEOUT"]);
            for def in &defs {
                table.add_row(vec![
                    def.name.clone(),
                    def.executor_type.to_string(),
                    def.command.clone().unwrap_or_default(),
                    def.timeout_seconds
                        .map(|t| format!("{t}s"))
                        .unwrap_or_default(),
                ]);
            }
            println!("{table}");
            Ok(())
        }
    }
}
