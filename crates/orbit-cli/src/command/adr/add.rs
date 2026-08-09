use std::path::PathBuf;

use clap::{ArgAction, Args};
use orbit_core::OrbitRuntime;
use serde_json::{Map, Value};

use crate::command::{CommandOut, CommandOutput, Execute, Payload};

use super::support::{resolve_body, response_id};

#[derive(Args)]
#[command(
    after_help = "The body follows the ADR template: `## Context`, `## Decision`, and a \
`## Consequences` list carrying at least one `Cost:` bullet. New ADRs are always created \
`proposed`; promote one with `orbit adr update <id> --status accepted --related-task <task-id>`."
)]
pub struct AdrAddArgs {
    /// ADR title (short noun phrase)
    #[arg(long)]
    pub title: String,
    /// ADR body as markdown (inline)
    #[arg(long)]
    pub body: Option<String>,
    /// Read the ADR body from a file
    #[arg(long = "body-file")]
    pub body_file: Option<PathBuf>,
    /// Agent identity that owns the ADR (e.g. `claude`, `codex`). Defaults to the calling actor.
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
    /// Explicit agent family for provenance (`codex`, `claude`, `gemini`, `grok`)
    #[arg(long)]
    pub model: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for AdrAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let body = resolve_body(self.body, self.body_file, true)?.unwrap_or_default();

        let mut input = Map::new();
        input.insert("title".to_string(), Value::String(self.title));
        input.insert("body".to_string(), Value::String(body));
        if let Some(owner) = self.owner {
            input.insert("owner".to_string(), Value::String(owner));
        }
        if let Some(model) = self.model {
            input.insert("model".to_string(), Value::String(model));
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

        // ID allocation, the body template, and the local-vs-federated write
        // routing all belong to `orbit.adr.add`; this subcommand only shapes
        // argv into that tool's input.
        let value = runtime.run_tool("orbit.adr.add", Value::Object(input))?;

        if self.json {
            Ok(Payload::document(value).into())
        } else {
            println!("{}", response_id(&value));
            Ok(CommandOutput::Silent)
        }
    }
}
