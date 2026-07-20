use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::json;

use crate::command::Execute;

use super::output::learning_to_json;

#[derive(Args)]
pub struct LearningSupersedeArgs {
    /// Learning ID being superseded
    pub id: String,
    /// Replacement learning ID
    #[arg(long = "with")]
    pub with: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for LearningSupersedeArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        if let Some(value) = super::managed_tool(
            runtime,
            "orbit.learning.supersede",
            json!({"id": self.id.clone(), "with": self.with.clone()}),
        )? {
            if self.json {
                return crate::output::json::print_pretty(&value);
            }
            println!(
                "{} superseded by {}",
                value["old"]["id"].as_str().unwrap_or_default(),
                value["new"]["id"].as_str().unwrap_or_default()
            );
            return Ok(());
        }
        runtime.supersede_learning(&self.id, &self.with)?;
        let old = runtime.get_learning(&self.id)?;
        let new = runtime.get_learning(&self.with)?;

        if self.json {
            crate::output::json::print_pretty(&json!({
                "old": learning_to_json(&old),
                "new": learning_to_json(&new),
            }))
        } else {
            println!("{} superseded by {}", old.id, new.id);
            Ok(())
        }
    }
}
