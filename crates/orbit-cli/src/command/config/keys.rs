use clap::Args;
use orbit_config::CONFIG_KEY_REGISTRY;
use orbit_core::OrbitRuntime;
use serde_json::json;

use crate::command::{CommandOut, CommandOutput, Execute, Payload};

#[derive(Args)]
pub struct ConfigKeysArgs {
    #[arg(long)]
    pub json: bool,
}

impl Execute for ConfigKeysArgs {
    fn execute(self, _runtime: &OrbitRuntime) -> CommandOut {
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
            Ok(Payload::document(json!({ "keys": keys })).into())
        } else {
            for entry in CONFIG_KEY_REGISTRY {
                println!(
                    "{:<36}  {:<24}  {}",
                    entry.key, entry.value_type, entry.description
                );
            }
            Ok(CommandOutput::Silent)
        }
    }
}
