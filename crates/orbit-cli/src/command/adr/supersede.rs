use clap::Args;
use orbit_core::OrbitRuntime;
use serde_json::{Map, Value};

use crate::command::{CommandOut, CommandOutput, Execute, Payload};

use super::support::response_id;

#[derive(Args)]
#[command(
    after_help = "The replacement must already be `accepted`, and both ADRs must be local to \
this checkout — a federated one fails closed with `artifact_not_local` naming its owning worktree."
)]
pub struct AdrSupersedeArgs {
    /// Canonical ADR ID being superseded
    pub id: String,
    /// Replacement ADR ID (must already be accepted)
    #[arg(long = "with")]
    pub with: String,
    /// Explicit agent family for provenance (`codex`, `claude`, `gemini`, `grok`)
    #[arg(long)]
    pub model: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for AdrSupersedeArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let mut input = Map::new();
        input.insert("old_id".to_string(), Value::String(self.id));
        input.insert("new_id".to_string(), Value::String(self.with.clone()));
        if let Some(model) = self.model {
            input.insert("model".to_string(), Value::String(model));
        }

        // The bidirectional edge write and the accepted-replacement precondition
        // belong to `orbit.adr.supersede`.
        let value = runtime.run_tool("orbit.adr.supersede", Value::Object(input))?;

        if self.json {
            Ok(Payload::document(value).into())
        } else {
            println!("{} superseded by {}", response_id(&value), self.with);
            Ok(CommandOutput::Silent)
        }
    }
}
