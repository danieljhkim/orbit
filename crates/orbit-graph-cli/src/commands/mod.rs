use std::env;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use orbit_graph::{Graph, GraphError, SyncPolicy};
use orbit_graph_extract::SelectorParseError;
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

mod callees;
mod clean;
mod db_path;
mod deps;
mod impact;
mod implementors;
mod overview;
mod refs;
mod search;
mod show;
mod sync;
mod trace;
mod version;

#[derive(Debug, Parser)]
#[command(name = "orbit-graph-cli", about = "Query the Orbit graph index")]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

impl Cli {
    pub fn run(&self) -> Result<Value, CliError> {
        self.command.run()
    }
}

impl Command {
    /// Dispatch this subcommand against a freshly discovered worktree context
    /// and return the JSON payload the caller is expected to emit.
    ///
    /// Shared by the standalone `orbit-graph-cli` binary and the `orbit graph`
    /// subcommand wired into `orbit-cli`.
    pub fn run(&self) -> Result<Value, CliError> {
        let context = CommandContext::from_current_dir()?;
        match self {
            Command::Sync(command) => command.run(&context),
            Command::Search(command) => command.run(&context),
            Command::Show(command) => command.run(&context),
            Command::Refs(command) => command.run(&context),
            Command::Callees(command) => command.run(&context),
            Command::Impact(command) => command.run(&context),
            Command::Trace(command) => command.run(&context),
            Command::Overview(command) => command.run(&context),
            Command::Implementors(command) => command.run(&context),
            Command::Deps(command) => command.run(&context),
            Command::Version(command) => command.run(),
            Command::DbPath(command) => command.run(&context),
            Command::Clean(command) => command.run(&context),
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Sync(sync::SyncCommand),
    Search(search::SearchCommand),
    Show(show::ShowCommand),
    Refs(refs::RefsCommand),
    Callees(callees::CalleesCommand),
    Impact(impact::ImpactCommand),
    Trace(trace::TraceCommand),
    Overview(overview::OverviewCommand),
    Implementors(implementors::ImplementorsCommand),
    Deps(deps::DepsCommand),
    Version(version::VersionCommand),
    DbPath(db_path::DbPathCommand),
    Clean(clean::CleanCommand),
}

pub(crate) struct CommandContext {
    worktree_root: PathBuf,
}

impl CommandContext {
    fn from_current_dir() -> Result<Self, CliError> {
        let current_dir = env::current_dir().map_err(CliError::CurrentDir)?;
        let worktree_root = git2::Repository::discover(current_dir.as_path())
            .ok()
            .and_then(|repo| repo.workdir().map(PathBuf::from))
            .unwrap_or(current_dir);
        Ok(Self { worktree_root })
    }

    pub(crate) fn open_graph(&self) -> Result<Graph, CliError> {
        Graph::open(self.worktree_root.as_path(), SyncPolicy::Manual).map_err(CliError::Graph)
    }
}

pub(crate) fn json_value<T: Serialize>(value: T) -> Result<Value, CliError> {
    serde_json::to_value(value).map_err(CliError::Json)
}

#[derive(Debug, Error)]
pub enum CliError {
    #[error(transparent)]
    Clap(clap::Error),
    #[error("failed to determine current directory: {0}")]
    CurrentDir(std::io::Error),
    #[error(transparent)]
    Graph(#[from] GraphError),
    #[error(transparent)]
    Selector(#[from] SelectorParseError),
    #[error("failed to serialize JSON: {0}")]
    Json(serde_json::Error),
    #[error("failed to write JSON to stdout: {0}")]
    Stdout(std::io::Error),
}

impl CliError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Clap(_) => "argument_error",
            Self::CurrentDir(_) => "current_dir_error",
            Self::Graph(_) => "graph_error",
            Self::Selector(_) => "selector_parse_error",
            Self::Json(_) => "json_error",
            Self::Stdout(_) => "stdout_error",
        }
    }

    pub fn details(&self) -> Option<&str> {
        match self {
            Self::Graph(GraphError::InvalidData { reason, .. }) => Some(reason.as_str()),
            Self::Graph(GraphError::Io { reason, .. }) => Some(reason.as_str()),
            Self::Graph(GraphError::Sqlite { reason, .. }) => Some(reason.as_str()),
            Self::Graph(GraphError::Unimplemented) => None,
            _ => None,
        }
    }
}
