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
//! ORB-10479: `orbit.adr.restore` (ORB-10538) is registered inactive for the
//! same reason — exact-id repair is an operator surface, not an agent one.
//! Registering it inactive also puts it out of reach of `orbit tool run`,
//! which gates on `ensure_tool_agent_facing`, so the tool needs this
//! subcommand to be reachable at all.
//!
//! Mirrors the shape of `orbit docs` (ORB-00280): one-file parent command
//! with `list` and `show` subcommands and a `--json` toggle. Add new ADR
//! subcommands here as they get promoted to the CLI surface.

use std::fs;
use std::path::PathBuf;

use clap::{ArgAction, Args, Subcommand};
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
    /// Show one ADR, including its body and artifact origin
    Show(AdrShowArgs),
    /// Restore an unreadable ADR at its exact existing allocation
    Restore(AdrRestoreArgs),
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
    /// Include allocated federated ADRs whose body files are not locally readable as remote stubs
    #[arg(long = "include-remote")]
    pub include_remote: bool,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct AdrShowArgs {
    /// Canonical ADR ID (for example `ADR-0259`)
    pub id: String,
    /// Output as JSON, including typed unavailable/not-found errors
    #[arg(long)]
    pub json: bool,
}

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

impl Execute for AdrCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self.command {
            AdrSubcommand::List(args) => args.execute(runtime),
            AdrSubcommand::Show(args) => args.execute(runtime),
            AdrSubcommand::Restore(args) => args.execute(runtime),
        }
    }
}

impl Execute for AdrRestoreArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let body = match (self.body, self.body_file) {
            (Some(_), Some(_)) => {
                return Err(OrbitError::InvalidInput(
                    "specify exactly one of `--body` and `--body-file`".to_string(),
                ));
            }
            (Some(body), None) => body,
            (None, Some(path)) => fs::read_to_string(&path)
                .map_err(|e| OrbitError::Io(format!("read body file {}: {e}", path.display())))?,
            (None, None) => {
                return Err(OrbitError::InvalidInput(
                    "specify exactly one of `--body` and `--body-file`".to_string(),
                ));
            }
        };

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

        // Same bypass as `list` above: `runtime.run_tool` skips
        // `ensure_tool_agent_facing` (which rejects the inactive
        // `orbit.adr.restore`) while keeping the tool's allocation guards —
        // missing allocation, still-readable artifact, lifecycle collision,
        // and the compare-and-set on the allocation row.
        let value = runtime.run_tool("orbit.adr.restore", Value::Object(input))?;

        if self.json {
            crate::output::json::print_pretty(&value)
        } else {
            println!("{}", value["id"].as_str().unwrap_or_default());
            Ok(())
        }
    }
}

impl Execute for AdrShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let value = runtime.run_tool("orbit.adr.show", serde_json::json!({ "id": self.id }))?;

        let _ = self.json;
        crate::output::json::print_pretty(&value)
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
