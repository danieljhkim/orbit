use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "Manage task templates")]
pub struct TaskTemplatesCommand {
    #[command(subcommand)]
    pub command: TaskTemplatesSubcommand,
}

impl Execute for TaskTemplatesCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum TaskTemplatesSubcommand {
    /// List available task templates (built-in and user-defined)
    List(TaskTemplatesListArgs),
}

impl Execute for TaskTemplatesSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            TaskTemplatesSubcommand::List(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct TaskTemplatesListArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskTemplatesListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let templates = runtime.list_task_templates()?;

        if self.json {
            let json_templates: Vec<Value> = templates
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "task_type": t.task_type.to_string(),
                        "priority": t.priority.to_string(),
                        "description_template": t.description_template,
                        "plan_template": t.plan_template,
                        "instructions_template": t.instructions_template,
                        "builtin": t.builtin,
                    })
                })
                .collect();
            crate::output::json::print_pretty(&Value::Array(json_templates))
        } else {
            use comfy_table::Cell;
            let mut table = crate::output::table::build_table(&[
                "NAME",
                "TYPE",
                "PRIORITY",
                "SOURCE",
                "DESCRIPTION",
            ]);
            for t in &templates {
                let source = if t.builtin { "built-in" } else { "user" };
                table.add_row(vec![
                    Cell::new(&t.name),
                    Cell::new(t.task_type.to_string()),
                    Cell::new(t.priority.to_string()),
                    Cell::new(source),
                    Cell::new(&t.description),
                ]);
            }
            println!("{table}");
            Ok(())
        }
    }
}
