use clap::Args;
use orbit_core::OrbitRuntime;
use serde_json::Value;

use crate::command::{CommandOut, Execute, Payload};

use super::support::executor_def_json;

#[derive(Args)]
pub struct ExecutorListArgs {
    #[arg(long)]
    pub json: bool,
}

impl Execute for ExecutorListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let defs = runtime.list_executor_defs()?;
        let values: Vec<Value> = defs.iter().map(executor_def_json).collect();

        use crate::output::table::{Column, Table};
        // `orbit executor show <name>` prints the untruncated command.
        let mut table = Table::new(vec![
            Column::new("NAME").fixed(),
            Column::new("TYPE").fixed(),
            Column::new("COMMAND").path(),
            Column::new("TIMEOUT (s)").number(),
        ])
        .empty_message("no executors defined");
        for def in &defs {
            table.add_row(vec![
                def.name.clone(),
                def.executor_type.to_string(),
                def.command.clone().unwrap_or_else(|| "-".to_string()),
                def.timeout_seconds
                    .map(|seconds| seconds.to_string())
                    .unwrap_or_else(|| "-".to_string()),
            ]);
        }
        Ok(Payload::list(values, table).into())
    }
}
