use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "Manage executor definitions")]
pub struct ExecutorCommand {
    #[command(subcommand)]
    pub command: ExecutorSubcommand,
}

impl Execute for ExecutorCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum ExecutorSubcommand {
    /// List all executor definitions
    List(ExecutorListArgs),
    /// Show a specific executor definition
    Show(ExecutorShowArgs),
}

impl Execute for ExecutorSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            ExecutorSubcommand::List(args) => args.execute(runtime),
            ExecutorSubcommand::Show(args) => args.execute(runtime),
        }
    }
}

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
                    def.executor_type.clone(),
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

fn executor_def_json(def: &orbit_core::ExecutorDef) -> Value {
    json!({
        "name": def.name,
        "executor_type": def.executor_type,
        "command": def.command,
        "args": def.args,
        "stdout_format": def.stdout_format,
        "timeout_seconds": def.timeout_seconds,
        "env": def.env,
        "created_at": def.created_at.to_rfc3339(),
        "updated_at": def.updated_at.to_rfc3339(),
    })
}
