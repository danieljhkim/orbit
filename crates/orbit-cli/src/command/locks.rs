//! `orbit task locks ...` — task file-lock administration.
//!
//! `list` renders the file-lock projection over active (in-progress/review)
//! tasks and any live reservations. `release` clears a stale reservation by
//! ID.
//!
//! Task lock reservations auto-release in workflow pipelines; `release` is the
//! operator escape hatch for a stale reservation that wedges a run. The
//! underlying `orbit.task.locks` / `orbit.task.locks.release` tools are
//! inactive on the agent MCP surface, so both reach them through the admin
//! `runtime.run_tool` bypass (mirrors `orbit adr list`, ORB-00289).

use clap::{Args, Subcommand};
use orbit_core::OrbitRuntime;
use serde_json::{Map, Value, json};

use crate::command::task::output::format_task_locks;
use crate::command::{CommandOut, CommandOutput, Execute, Payload, require_confirmation};

#[derive(Args)]
#[command(about = "Inspect and release task file locks")]
pub struct LocksCommand {
    #[command(subcommand)]
    pub command: LocksSubcommand,
}

impl Execute for LocksCommand {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum LocksSubcommand {
    /// Show files locked by active (in-progress/review) tasks and reservations
    List(LocksListArgs),
    /// Release a stale task lock reservation (operator/admin escape hatch)
    Release(LocksReleaseArgs),
}

impl Execute for LocksSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        match self {
            LocksSubcommand::List(args) => args.execute(runtime),
            LocksSubcommand::Release(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
#[command(about = "Show files locked by active (in-progress/review) tasks and reservations")]
pub struct LocksListArgs {
    /// Output the lock projection as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for LocksListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let locks = runtime.run_tool("orbit.task.locks", json!({}))?;
        if self.json {
            Ok(Payload::document(locks).into())
        } else {
            print!("{}", format_task_locks(&locks));
            Ok(CommandOutput::Silent)
        }
    }
}

#[derive(Args)]
#[command(about = "Release a stale task lock reservation (operator/admin escape hatch)")]
pub struct LocksReleaseArgs {
    /// Reservation ID to release (see `orbit task locks list --json`)
    pub reservation_id: String,
    /// Confirm release of the reservation
    #[arg(long)]
    pub confirm: bool,
}

impl Execute for LocksReleaseArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        require_confirmation(self.confirm, "task lock release")?;
        let mut input = Map::new();
        input.insert(
            "reservation_id".to_string(),
            Value::String(self.reservation_id),
        );
        let value = runtime.run_tool("orbit.task.locks.release", Value::Object(input))?;
        Ok(Payload::document(value).into())
    }
}
