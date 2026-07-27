use clap::Args;
use orbit_core::config::CONFIG_KEY_REGISTRY;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::json;

use crate::command::Execute;

#[derive(Args)]
pub struct ConfigKeysArgs {
    #[arg(long)]
    pub json: bool,
}

impl Execute for ConfigKeysArgs {
    fn execute(self, _runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        if self.json {
            let keys: Vec<_> = CONFIG_KEY_REGISTRY
                .iter()
                .map(|entry| {
                    json!({
                        "key": entry.key,
                        "type": entry.value_type,
                        "description": entry.description,
                    })
                })
                .collect();
            crate::output::json::print_pretty(&json!({ "keys": keys }))
        } else {
            for entry in CONFIG_KEY_REGISTRY {
                println!(
                    "{:<36}  {:<24}  {}",
                    entry.key, entry.value_type, entry.description
                );
            }
            Ok(())
        }
    }
}
