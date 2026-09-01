//! `orbit task locks ...` — task file-lock administration.
//!
//! `list` renders the file-lock projection over active (in-progress/review)
//! tasks and any live reservations. `contention` looks the other way, at the
//! pending backlog, and reports which selectors will serialize it. `release`
//! clears a stale reservation by ID.
//!
//! Task lock reservations auto-release in workflow pipelines; `release` is the
//! operator escape hatch for a stale reservation that wedges a run. The
//! underlying `orbit.task.locks` / `orbit.task.locks.release` tools are
//! inactive on the agent MCP surface, so both reach them through the admin
//! `runtime.run_tool` bypass (mirrors `orbit adr list`, ORB-00289).

use clap::{Args, Subcommand};
use orbit_core::{OrbitRuntime, TaskStatus};
use serde_json::{Map, Value, json};

use crate::command::task::output::format_task_locks;
use crate::command::{Block, CommandOut, CommandOutput, Execute, Payload, require_confirmation};
use crate::output::table::{Column, Table};

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
    /// Show which files the pending backlog collides on (what caps parallelism)
    Contention(LocksContentionArgs),
    /// Release a stale task lock reservation (operator/admin escape hatch)
    Release(LocksReleaseArgs),
}

impl Execute for LocksSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        match self {
            LocksSubcommand::List(args) => args.execute(runtime),
            LocksSubcommand::Contention(args) => args.execute(runtime),
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

/// Hotspot rows rendered by default. Long enough to show a pattern, short
/// enough that the tail of singletons does not bury it.
const DEFAULT_CONTENTION_LIMIT: usize = 10;

#[derive(Args)]
#[command(
    about = "Show which files the pending backlog collides on (what caps parallelism)",
    after_help = "Examples:\n  orbit task locks contention\n  orbit task locks contention --limit 25\n  orbit task locks contention --json\n\n\
                  TASKS counts pending tasks whose lock surface overlaps that selector; a\n\
                  selector claimed by one task constrains nothing and is not listed.\n\n\
                  GROUPS counts clusters of pending tasks linked by overlapping surfaces.\n\
                  Tasks in different groups can never block each other, so the group count\n\
                  is a floor on how many dispatch can admit at once — not a ceiling, since\n\
                  two tasks inside one group are often compatible and merely linked through\n\
                  a third.\n\n\
                  Surfaces are the ones conflict admission reserves: declared selectors,\n\
                  pruned of paths that no longer exist, unioned across descendants for an\n\
                  epic root. A task declaring nothing locks nothing and is counted apart."
)]
pub struct LocksContentionArgs {
    /// Maximum hotspot rows to show. Default 10.
    #[arg(long, default_value_t = DEFAULT_CONTENTION_LIMIT)]
    pub limit: usize,
    /// Output the contention report as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for LocksContentionArgs {
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let report = runtime.task_lock_contention(&[TaskStatus::Backlog])?;

        let mut table = Table::new(vec![
            Column::new("SELECTOR").fixed(),
            Column::new("TASKS").number(),
        ])
        .keep_all_columns()
        .empty_message("no selector is claimed by more than one pending task");
        for hotspot in report.hotspots.iter().take(self.limit) {
            table.add_row(vec![hotspot.selector.clone(), hotspot.tasks().to_string()]);
        }

        let doc = json!({
            "hotspots": report
                .hotspots
                .iter()
                .map(|hotspot| json!({
                    "selector": hotspot.selector,
                    "tasks": hotspot.tasks(),
                    "task_ids": hotspot.task_ids,
                }))
                .collect::<Vec<_>>(),
            "pending": {
                "total": report.pending(),
                "constrained": report.constrained,
                "unconstrained": report.unconstrained,
            },
            "groups": report.groups,
            "largest_group": report.largest_group,
            "parallel_floor": report.parallel_floor(),
        });

        Ok(Payload::blocks(
            doc,
            vec![
                Block::table(table),
                Block::text(contention_summary(&report, self.limit)),
            ],
        )
        .into())
    }
}

/// One line stating what the numbers mean for dispatch.
fn contention_summary(report: &orbit_core::LockContentionReport, limit: usize) -> String {
    if report.pending() == 0 {
        return "no pending tasks to analyze".to_string();
    }
    if report.hotspots.is_empty() {
        return format!(
            "{} pending tasks, no contention — no selector is claimed by more than one",
            report.pending()
        );
    }
    let shown = report.hotspots.len().min(limit);
    let elided = if shown < report.hotspots.len() {
        format!(", top {shown} shown")
    } else {
        String::new()
    };
    format!(
        "{} pending tasks ({} declare no files), {} contended selector(s){} — at least {} can run in parallel, largest cluster chains {}",
        report.pending(),
        report.unconstrained,
        report.hotspots.len(),
        elided,
        report.parallel_floor(),
        report.largest_group,
    )
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
