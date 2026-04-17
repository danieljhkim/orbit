use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "Manage declarative policy definitions")]
pub struct PolicyCommand {
    #[command(subcommand)]
    pub command: PolicySubcommand,
}

impl Execute for PolicyCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum PolicySubcommand {
    /// List all policy definitions
    List(PolicyListArgs),
    /// Show a specific policy definition
    Show(PolicyShowArgs),
}

impl Execute for PolicySubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            PolicySubcommand::List(args) => args.execute(runtime),
            PolicySubcommand::Show(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct PolicyListArgs {
    #[arg(long)]
    pub json: bool,
}

impl Execute for PolicyListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let defs = runtime.list_policy_defs()?;
        if self.json {
            let values: Vec<Value> = defs
                .iter()
                .map(|d| {
                    json!({
                        "name": d.name,
                        "description": d.description,
                        "created_at": d.created_at.to_rfc3339(),
                        "updated_at": d.updated_at.to_rfc3339(),
                    })
                })
                .collect();
            crate::output::json::print_pretty(&Value::Array(values))
        } else {
            if defs.is_empty() {
                println!("No policy definitions found.");
                return Ok(());
            }
            let mut table = crate::output::table::build_table(&["NAME", "DESCRIPTION", "UPDATED"]);
            for def in &defs {
                table.add_row(vec![
                    def.name.clone(),
                    def.description.clone().unwrap_or_default(),
                    def.updated_at.format("%Y-%m-%d %H:%M").to_string(),
                ]);
            }
            println!("{table}");
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct PolicyShowArgs {
    pub name: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for PolicyShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let def = runtime
            .get_policy_def(&self.name)?
            .ok_or_else(|| OrbitError::InvalidInput(format!("policy not found: {}", self.name)))?;

        if self.json {
            let value =
                serde_json::to_value(&def).map_err(|e| OrbitError::Execution(e.to_string()))?;
            crate::output::json::print_pretty(&value)
        } else {
            println!("Name:        {}", def.name);
            if let Some(desc) = &def.description {
                println!("Description: {desc}");
            }
            println!("Created:     {}", def.created_at.to_rfc3339());
            println!("Updated:     {}", def.updated_at.to_rfc3339());

            if let Some(fs) = &def.filesystem {
                println!("\nFilesystem:");
                if !fs.allow_write.is_empty() {
                    println!("  allow_write: {}", fs.allow_write.join(", "));
                }
                if !fs.deny_write.is_empty() {
                    println!("  deny_write:  {}", fs.deny_write.join(", "));
                }
            }
            if let Some(net) = &def.network {
                println!("\nNetwork:");
                println!("  mode: {:?}", net.mode);
                if !net.allow_hosts.is_empty() {
                    println!("  allow_hosts: {}", net.allow_hosts.join(", "));
                }
                if !net.deny_hosts.is_empty() {
                    println!("  deny_hosts:  {}", net.deny_hosts.join(", "));
                }
            }
            if let Some(proc) = &def.process {
                println!("\nProcess:");
                if !proc.allow_commands.is_empty() {
                    println!("  allow_commands: {}", proc.allow_commands.join(", "));
                }
                if !proc.deny_commands.is_empty() {
                    println!("  deny_commands:  {}", proc.deny_commands.join(", "));
                }
            }
            if let Some(approval) = &def.approval {
                println!("\nApproval:");
                if !approval.require_for.is_empty() {
                    println!("  require_for: {}", approval.require_for.join(", "));
                }
            }
            if let Some(tools) = &def.tools {
                println!("\nTools:");
                if !tools.allow.is_empty() {
                    println!("  allow: {}", tools.allow.join(", "));
                }
                if !tools.deny.is_empty() {
                    println!("  deny:  {}", tools.deny.join(", "));
                }
            }
            Ok(())
        }
    }
}
