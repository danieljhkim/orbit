use clap::Args;
use orbit_graph::clean_old_databases;
use serde::Serialize;

use super::{CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub(crate) struct CleanCommand;

impl CleanCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let report = clean_old_databases(context.worktree_root.as_path())?;
        json_value(CleanOutput {
            graph_dir: report.graph_dir.display().to_string(),
            deleted: report
                .deleted
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
        })
    }
}

#[derive(Debug, Serialize)]
struct CleanOutput {
    graph_dir: String,
    deleted: Vec<String>,
}
