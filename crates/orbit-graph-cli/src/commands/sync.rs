use std::time::Duration;

use clap::Args;
use orbit_graph::SyncMode;
use serde::Serialize;

use super::{CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub(crate) struct SyncCommand {
    #[arg(long)]
    full: bool,
}

impl SyncCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let graph = context.open_graph()?;
        let report = graph.sync(if self.full {
            SyncMode::Full
        } else {
            SyncMode::Auto
        })?;
        json_value(SyncOutput {
            files_indexed: report.files_indexed,
            files_changed: report.files_changed,
            files_removed: report.files_removed,
            duration_ms: duration_millis(report.duration),
        })
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
