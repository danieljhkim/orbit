use clap::Args;
use orbit_core::config::{CONFIG_KEY_REGISTRY, load_effective_config};
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::json;

use crate::command::{CommandOut, CommandOutput, Execute, Payload};

use super::support::{ConfigScopeArg, open_store_for_scope};

#[derive(Args)]
pub struct ConfigGetArgs {
    /// Dotted config.toml key, e.g. workflow.base_branch
    pub key: String,
    #[arg(long, value_enum, default_value_t = ConfigScopeArg::Effective)]
    pub scope: ConfigScopeArg,
    #[arg(long)]
    pub json: bool,
}

impl Execute for ConfigGetArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        if self.scope == ConfigScopeArg::Effective {
            let effective = load_effective_config(&runtime.global_root(), &runtime.shared_root())?;
            let value = effective.value_for(&self.key).ok_or_else(|| {
                OrbitError::invalid_input_with_suggestions(
                    format!("unknown config key '{}'", self.key),
                    CONFIG_KEY_REGISTRY
                        .iter()
                        .map(|descriptor| descriptor.key.to_string())
                        .collect(),
                )
            })?;
            if self.json {
                return Ok(Payload::document(json!({
                    "key": self.key,
                    "scope": "effective",
                    "value": value,
                }))
                .into());
            }
            println!("{}", format_value_for_display(&value));
            return Ok(CommandOutput::Silent);
        }

        let store = open_store_for_scope(runtime, self.scope)?;
        let value = store.effective_value(&self.key)?;

        if self.json {
            Ok(Payload::document(json!({
                "key": self.key,
                "scope": store.scope().label(),
                "path": store.path().to_string_lossy(),
                "value": value,
            }))
            .into())
        } else {
            println!("{}", format_value_for_display(&value));
            Ok(CommandOutput::Silent)
        }
    }
}

fn format_value_for_display(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}
