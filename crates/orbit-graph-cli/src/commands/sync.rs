use std::time::Duration;

use clap::Args;
use orbit_graph::{GraphQueryKind, SyncMode};
use serde::Serialize;

use super::{BackendArg, CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub(crate) struct SyncCommand {
    #[arg(long)]
    full: bool,
    #[arg(long, value_enum)]
    backend: Option<BackendArg>,
}

impl SyncCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let full = self.full;
        let worktree = context.worktree_root.clone();
        context.route_query(
            self.backend,
            GraphQueryKind::Sync,
            move || {
                let graph =
                    orbit_graph::Graph::open(worktree.as_path(), orbit_graph::SyncPolicy::Manual)
                        .map_err(CliError::Graph)?;
                let report = graph.sync(if full { SyncMode::Full } else { SyncMode::Auto })?;
                json_value(SyncOutput {
                    files_indexed: report.files_indexed,
                    files_changed: report.files_changed,
                    files_removed: report.files_removed,
                    duration_ms: duration_millis(report.duration),
                })
            },
            || context.run_legacy_sync(full),
        )
    }
}

#[derive(Debug, Serialize)]
struct SyncOutput {
    files_indexed: usize,
    files_changed: usize,
    files_removed: usize,
    duration_ms: u128,
}

fn duration_millis(duration: Duration) -> u128 {
    duration.as_millis()
}
