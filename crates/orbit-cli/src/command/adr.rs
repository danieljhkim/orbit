//! `orbit adr ...` CLI surface.
//!
//! ORB-00289: `orbit.adr.list` was inactivated on the agent MCP surface
//! (agents discover ADRs via `orbit search --kind adr`). The CLI/admin
//! path still needs a way to list ADRs — including features like
//! `--include-remote` that don't surface through `orbit search` — so this
//! subcommand reaches the underlying tool through `runtime.run_tool`,
//! which bypasses `ensure_tool_agent_facing` while preserving the tool's
//! input parsing and filter semantics.
//!
//! Mirrors the shape of `orbit docs` (ORB-00280): one-file parent command
//! with a `list` subcommand and a `--json` toggle. Add new ADR
//! subcommands here as they get promoted to the CLI surface.

use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Map, Value};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "List and inspect Architecture Decision Records")]
pub struct AdrCommand {
    #[command(subcommand)]
    pub command: AdrSubcommand,
}

#[derive(Subcommand)]
pub enum AdrSubcommand {
    /// List ADRs with optional filters
    List(AdrListArgs),
}

#[derive(Args)]
pub struct AdrListArgs {
    /// Filter by status: proposed | accepted | superseded | deleted
    #[arg(long)]
    pub status: Option<String>,
    /// Filter by owner (e.g. `claude`)
    #[arg(long)]
    pub owner: Option<String>,
    /// Filter by feature folder name in `related_features`
    #[arg(long)]
    pub feature: Option<String>,
    /// Filter by Orbit task ID in `related_tasks`
    #[arg(long = "task-id")]
    pub task_id: Option<String>,
    /// Filter by legacy ID alias in `legacy_ids`
    #[arg(long = "legacy-id")]
    pub legacy_id: Option<String>,
    /// Filter by free-form ADR tag (case-insensitive)
    #[arg(long)]
    pub tag: Option<String>,
    /// Filter by repo-relative path contained by any ADR `paths` glob
    #[arg(long)]
    pub path: Option<String>,
    /// When set, return only ADRs with `legacy_validation = warned`
    #[arg(long = "validation-warned")]
    pub validation_warned: bool,
    /// Include allocation rows whose body files are not locally readable as remote stubs
    #[arg(long = "include-remote")]
    pub include_remote: bool,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for AdrCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self.command {
            AdrSubcommand::List(args) => args.execute(runtime),
        }
    }
}

impl Execute for AdrListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let mut input = Map::new();
        if let Some(value) = self.status {
            input.insert("status".to_string(), Value::String(value));
        }
        if let Some(value) = self.owner {
            input.insert("owner".to_string(), Value::String(value));
        }
        if let Some(value) = self.feature {
            input.insert("feature".to_string(), Value::String(value));
        }
        if let Some(value) = self.task_id {
            input.insert("task_id".to_string(), Value::String(value));
        }
        if let Some(value) = self.legacy_id {
            input.insert("legacy_id".to_string(), Value::String(value));
        }
        if let Some(value) = self.tag {
            input.insert("tag".to_string(), Value::String(value));
        }
        if let Some(value) = self.path {
            input.insert("path".to_string(), Value::String(value));
        }
        if self.validation_warned {
            input.insert("validation_warned".to_string(), Value::Bool(true));
        }
        if self.include_remote {
            input.insert("include_remote".to_string(), Value::Bool(true));
        }

        // ORB-00289: `runtime.run_tool` bypasses `ensure_tool_agent_facing`
        // (which now rejects `orbit.adr.list` on the agent surface) while
        // still going through the tool's input parsing and filter
        // semantics. This is the same path used by `orbit-dashboard`'s
        // `run_adr_tool` helper.
        let value = runtime.run_tool("orbit.adr.list", Value::Object(input))?;

        // The tool already returns the canonical ADR envelope shape; emit
        // it pretty-printed in both modes for now. A table renderer can be
        // added later if/when a richer non-JSON UX is needed.
        let _ = self.json;
        crate::output::json::print_pretty(&value)
    }
}
