use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

use crate::command::Execute;

use super::support::sorted_profile_names;

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
                        "fs_profiles": sorted_profile_names(d),
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
            let mut table = crate::output::table::build_table(&[
                "NAME",
                "DESCRIPTION",
                "FSPROFILES",
                "UPDATED",
            ]);
            for def in &defs {
                table.add_row(vec![
                    def.name.clone(),
                    def.description.clone().unwrap_or_default(),
                    sorted_profile_names(def).join(", "),
                    def.updated_at.format("%Y-%m-%d %H:%M").to_string(),
                ]);
            }
            println!("{table}");
            Ok(())
        }
    }
}
