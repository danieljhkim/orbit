use std::path::PathBuf;

use clap::{ArgAction, Args};
use orbit_core::OrbitRuntime;
use serde_json::{Map, Value};

use crate::command::{CommandOut, CommandOutput, Execute, Payload};

use super::support::{resolve_body, response_id};

#[derive(Args)]
pub struct AdrRestoreArgs {
    /// Exact existing canonical ADR allocation to restore (e.g. `ADR-0184`)
    #[arg(long)]
    pub id: String,
    /// ADR title (short noun phrase)
    #[arg(long)]
    pub title: String,
    /// ADR body as markdown (inline)
    #[arg(long)]
    pub body: Option<String>,
    /// Read the ADR body from a file
    #[arg(long = "body-file")]
    pub body_file: Option<PathBuf>,
    /// Agent identity that owns the ADR (e.g. `claude`, `codex`)
    #[arg(long)]
    pub owner: Option<String>,
    /// Feature folder this decision touches. Repeat for multiple.
    #[arg(long = "related-feature", action = ArgAction::Append)]
    pub related_features: Vec<String>,
    /// Orbit task ID that proposed or shipped the decision. Repeat for multiple.
    #[arg(long = "related-task", action = ArgAction::Append)]
    pub related_tasks: Vec<String>,
    /// Free-form ADR label. Repeat for multiple.
    #[arg(long = "tag", action = ArgAction::Append)]
    pub tags: Vec<String>,
    /// Repo-relative glob constrained by this ADR. Repeat for multiple.
    #[arg(long = "path", action = ArgAction::Append)]
    pub paths: Vec<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for AdrRestoreArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let body = resolve_body(self.body, self.body_file, true)?.unwrap_or_default();

        let mut input = Map::new();
        input.insert("id".to_string(), Value::String(self.id));
        input.insert("title".to_string(), Value::String(self.title));
        input.insert("body".to_string(), Value::String(body));
        if let Some(owner) = self.owner {
            input.insert("owner".to_string(), Value::String(owner));
        }
        for (key, values) in [
            ("related_features", self.related_features),
            ("related_tasks", self.related_tasks),
            ("tags", self.tags),
            ("paths", self.paths),
        ] {
            if !values.is_empty() {
                input.insert(key.to_string(), Value::from(values));
            }
        }

        // Same bypass as `list`: `runtime.run_tool` skips
        // `ensure_tool_agent_facing` (which rejects the inactive
        // `orbit.adr.restore`) while keeping the tool's allocation guards —
        // missing allocation, still-readable artifact, lifecycle collision,
        // and the compare-and-set on the allocation row.
        let value = runtime.run_tool("orbit.adr.restore", Value::Object(input))?;

        if self.json {
            Ok(Payload::document(value).into())
        } else {
            println!("{}", response_id(&value));
            Ok(CommandOutput::Silent)
        }
    }
}
