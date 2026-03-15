use clap::{Args, Subcommand};
use orbit_core::IdentityRole;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::json;

use crate::command::Execute;

#[derive(Args)]
pub struct IdentityCommand {
    #[command(subcommand)]
    pub command: IdentitySubcommand,
}

impl Execute for IdentityCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum IdentitySubcommand {
    List(IdentityListArgs),
    Show(IdentityShowArgs),
}

impl Execute for IdentitySubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            IdentitySubcommand::List(args) => args.execute(runtime),
            IdentitySubcommand::Show(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct IdentityListArgs {
    #[arg(long)]
    pub json: bool,
    /// Filter by identity role (e.g. engineer, architect, reviewer, member, leader, product-designer, ceo).
    #[arg(long, value_name = "ROLE")]
    pub role: Option<String>,
}

impl Execute for IdentityListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let role = self
            .role
            .as_deref()
            .map(|r| {
                r.parse::<IdentityRole>()
                    .map_err(OrbitError::IdentityValidation)
            })
            .transpose()?;
        let identities = runtime.list_identities_filtered(role)?;
        if self.json {
            let values: Vec<_> = identities
                .iter()
                .map(|identity| json!({ "id": identity.id }))
                .collect();
            crate::output::json::print_pretty(&serde_json::Value::Array(values))
        } else {
            println!("{:<24} NAME", "ID");
            for identity in &identities {
                println!("{:<24} {}", identity.id, identity.name);
            }
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct IdentityShowArgs {
    pub identity_id: String,
    #[arg(long)]
    pub json: bool,
}

impl Execute for IdentityShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let identity = runtime.show_identity(&self.identity_id)?;
        if self.json {
            crate::output::json::print_pretty(&json!({
                "id": identity.id,
                "name": identity.name,
                "role": identity.role.to_string(),
                "personality": identity.personality,
                "behavior": identity.behavior,
            }))
        } else {
            println!("Identity: {}", identity.id);
            println!("Name:     {}", identity.name);
            println!("Role:     {}", identity.role);
            if !identity.personality.is_empty() {
                println!("\nPersonality:");
                for (k, v) in &identity.personality {
                    println!("  {k}: {v}");
                }
            }
            if !identity.behavior.is_empty() {
                println!("\nBehavior:");
                for (k, v) in &identity.behavior {
                    println!("  {k}: {v}");
                }
            }
            Ok(())
        }
    }
}
