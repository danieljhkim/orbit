use clap::Args;
use orbit_core::OrbitRuntime;

use crate::command::{CommandOut, Execute, Payload};

#[derive(Args)]
pub struct AdrShowArgs {
    /// Canonical ADR ID (for example `ADR-0259`)
    pub id: String,
    /// Output as JSON, including typed unavailable/not-found errors
    #[arg(long)]
    pub json: bool,
}

impl Execute for AdrShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let value = runtime.run_tool("orbit.adr.show", serde_json::json!({ "id": self.id }))?;

        let _ = self.json;
        Ok(Payload::document(value).into())
    }
}
