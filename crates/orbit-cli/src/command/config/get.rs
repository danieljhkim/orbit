use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::json;

use crate::command::Execute;

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
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let store = open_store_for_scope(runtime, self.scope)?;
        let value = store.effective_value(&self.key)?;

        if self.json {
            crate::output::json::print_pretty(&json!({
                "key": self.key,
                "scope": store.scope().label(),
                "path": store.path().to_string_lossy(),
                "value": value,
            }))
        } else {
            println!("{}", format_value_for_display(&value));
            Ok(())
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
