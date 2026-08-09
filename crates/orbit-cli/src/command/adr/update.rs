use std::path::PathBuf;

use clap::{ArgAction, Args};
use orbit_core::OrbitRuntime;
use serde_json::{Map, Value};

use crate::command::{CommandOut, CommandOutput, Execute, Payload};

use super::support::{replacement_list, resolve_body, response_id};

/// ORB-10668: the lifecycle rules themselves live in `orbit.adr.update`, so
/// this text is the CLI's only copy of them — it is what makes the federated
/// path discoverable from `orbit adr update --help` rather than from a 409.
const UPDATE_AFTER_HELP: &str = "\
Lifecycle:
  proposed -> accepted   `--status accepted`, which requires at least one related
                         task on the resulting record (`--related-task ORB-00042`,
                         or one the ADR already carries).
  accepted -> proposed   rejected.
  -> superseded          not a status write; use `orbit adr supersede <old> --with <new>`.

Federated ADRs:
  An ADR whose bundle was authored in another checkout fails closed with
  `artifact_not_local` and reports the owning worktree in `artifact_origin`.
  Either run this command from that worktree, or bring the bundle into this one
  first with `orbit adr reconcile <id> --source-worktree <path>`.

List flags (`--related-feature`, `--related-task`, `--tag`, `--path`) replace the
stored list. Omit one to leave it unchanged; pass it once with an empty string
(`--tag \"\"`) to clear it.";

#[derive(Args)]
#[command(after_help = UPDATE_AFTER_HELP)]
pub struct AdrUpdateArgs {
    /// Canonical ADR ID (for example `ADR-0259`)
    pub id: String,
    /// New status: `proposed` | `accepted`
    #[arg(long)]
    pub status: Option<String>,
    /// New title
    #[arg(long)]
    pub title: Option<String>,
    /// New owner
    #[arg(long)]
    pub owner: Option<String>,
    /// Replacement body markdown (inline)
    #[arg(long)]
    pub body: Option<String>,
    /// Read the replacement body from a file
    #[arg(long = "body-file")]
    pub body_file: Option<PathBuf>,
    /// Replacement feature folder. Repeat for multiple.
    #[arg(long = "related-feature", action = ArgAction::Append)]
    pub related_features: Vec<String>,
    /// Replacement Orbit task ID. Repeat for multiple.
    #[arg(long = "related-task", action = ArgAction::Append)]
    pub related_tasks: Vec<String>,
    /// Replacement free-form ADR label. Repeat for multiple.
    #[arg(long = "tag", action = ArgAction::Append)]
    pub tags: Vec<String>,
    /// Replacement repo-relative applicability glob. Repeat for multiple.
    #[arg(long = "path", action = ArgAction::Append)]
    pub paths: Vec<String>,
    /// Replacement legacy ID alias. Repeat for multiple.
    #[arg(long = "legacy-id", action = ArgAction::Append)]
    pub legacy_ids: Vec<String>,
    /// Explicit agent family for provenance (`codex`, `claude`, `gemini`, `grok`)
    #[arg(long)]
    pub model: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for AdrUpdateArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let body = resolve_body(self.body, self.body_file, false)?;

        let mut input = Map::new();
        input.insert("id".to_string(), Value::String(self.id));
        for (key, value) in [
            ("status", self.status),
            ("title", self.title),
            ("owner", self.owner),
            ("body", body),
            ("model", self.model),
        ] {
            if let Some(value) = value {
                input.insert(key.to_string(), Value::String(value));
            }
        }
        for (key, values) in [
            ("related_features", self.related_features),
            ("related_tasks", self.related_tasks),
            ("tags", self.tags),
            ("paths", self.paths),
            ("legacy_ids", self.legacy_ids),
        ] {
            if let Some(values) = replacement_list(values) {
                input.insert(key.to_string(), Value::from(values));
            }
        }

        // Status parsing, the `proposed -> accepted` related-task rule, the
        // refusal of direct `superseded` writes, and the `artifact_not_local`
        // federation guard all stay in `orbit.adr.update`. The CLI must not
        // re-derive any of them, or the two surfaces would drift.
        let value = runtime.run_tool("orbit.adr.update", Value::Object(input))?;

        if self.json {
            Ok(Payload::document(value).into())
        } else {
            println!(
                "{} is now {}",
                response_id(&value),
                value["status"].as_str().unwrap_or("unknown")
            );
            Ok(CommandOutput::Silent)
        }
    }
}
