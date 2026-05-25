use clap::Args;
use serde::Serialize;

use super::{CliError, CommandContext, json_value};

#[derive(Debug, Args)]
pub(crate) struct DbPathCommand;

impl DbPathCommand {
    pub(crate) fn run(&self, context: &CommandContext) -> Result<serde_json::Value, CliError> {
        let graph = context.open_graph()?;
        let db_path = graph.db_path();
        json_value(DbPathOutput {
            path: db_path.path().display().to_string(),
            branch: db_path.branch().to_string(),
            extractor_version: db_path.extractor_version(),
        })
    }
}

#[derive(Debug, Serialize)]
struct DbPathOutput {
    path: String,
    branch: String,
    extractor_version: u32,
}
