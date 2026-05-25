use std::env;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;

use clap::{Parser, Subcommand, ValueEnum};
use orbit_graph::{
    Graph, GraphBackend, GraphBackendParseError, GraphError, GraphQueryKind, SyncPolicy,
    route_query,
};
use orbit_graph_extract::SelectorParseError;
use serde::Serialize;
use serde_json::{Value, json};
use thiserror::Error;

mod callees;
mod clean;
mod db_path;
mod impact;
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
        let context = CommandContext::from_current_dir()?;
        match &self.command {
            Command::Sync(command) => command.run(&context),
            Command::Search(command) => command.run(&context),
            Command::Show(command) => command.run(&context),
            Command::Refs(command) => command.run(&context),
            Command::Callees(command) => command.run(&context),
            Command::Impact(command) => command.run(&context),
            Command::Trace(command) => command.run(&context),
            Command::Version(command) => command.run(),
            Command::DbPath(command) => command.run(&context),
            Command::Clean(command) => command.run(&context),
        }
    }
}

#[derive(Debug, Subcommand)]
enum Command {
    Sync(sync::SyncCommand),
    Search(search::SearchCommand),
    Show(show::ShowCommand),
    Refs(refs::RefsCommand),
    Callees(callees::CalleesCommand),
    Impact(impact::ImpactCommand),
    Trace(trace::TraceCommand),
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

    pub(crate) fn route_query<N, L>(
        &self,
        backend: Option<BackendArg>,
        query: GraphQueryKind,
        run_new: N,
        run_legacy: L,
    ) -> Result<Value, CliError>
    where
        N: FnOnce() -> Result<Value, CliError> + Send,
        L: FnOnce() -> Result<Value, CliError> + Send,
    {
        let backend = GraphBackend::resolve(backend.map(BackendArg::into_graph))?;
        route_query(backend, query, run_new, run_legacy)
    }

    pub(crate) fn run_legacy_tool(&self, tool_name: &str, input: Value) -> Result<Value, CliError> {
        let orbit = env::var_os("ORBIT_LEGACY_CLI")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("orbit"));
        let output = ProcessCommand::new(&orbit)
            .current_dir(self.worktree_root.as_path())
            .args(["tool", "run", tool_name, "--full", "--input"])
            .arg(input.to_string())
            .output()
            .map_err(|source| CliError::LegacyProcess {
                command: orbit.display().to_string(),
                source,
            })?;
        if !output.status.success() {
            return Err(CliError::LegacyTool {
                command: format!("{} tool run {tool_name}", orbit.display()),
                status: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        serde_json::from_slice(&output.stdout).map_err(CliError::Json)
    }

    pub(crate) fn run_legacy_sync(&self, full: bool) -> Result<Value, CliError> {
        let orbit = env::var_os("ORBIT_LEGACY_CLI")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("orbit"));
        let subcommand = if full { "build" } else { "update" };
        let output = ProcessCommand::new(&orbit)
            .current_dir(self.worktree_root.as_path())
            .args(["graph", subcommand])
            .output()
            .map_err(|source| CliError::LegacyProcess {
                command: orbit.display().to_string(),
                source,
            })?;
        if !output.status.success() {
            return Err(CliError::LegacyTool {
                command: format!("{} graph {subcommand}", orbit.display()),
                status: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        Ok(json!({
            "backend": "legacy",
            "command": format!("graph {subcommand}"),
        }))
    }

    pub(crate) fn legacy_unavailable(&self, query: &'static str) -> Result<Value, CliError> {
        Err(CliError::LegacyUnavailable(query))
    }
}

pub(crate) fn json_value<T: Serialize>(value: T) -> Result<Value, CliError> {
    serde_json::to_value(value).map_err(CliError::Json)
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum BackendArg {
    Legacy,
    New,
    Both,
}

impl BackendArg {
    fn into_graph(self) -> GraphBackend {
        match self {
            Self::Legacy => GraphBackend::Legacy,
            Self::New => GraphBackend::New,
            Self::Both => GraphBackend::Both,
        }
    }
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
    Backend(#[from] GraphBackendParseError),
    #[error(transparent)]
    Selector(#[from] SelectorParseError),
    #[error("failed to serialize JSON: {0}")]
    Json(serde_json::Error),
    #[error("failed to write JSON to stdout: {0}")]
    Stdout(std::io::Error),
    #[error("failed to run legacy graph command `{command}`: {source}")]
    LegacyProcess {
        command: String,
        source: std::io::Error,
    },
    #[error("legacy graph command `{command}` failed")]
    LegacyTool {
        command: String,
        status: Option<i32>,
        stderr: String,
    },
    #[error("legacy graph backend does not support `{0}`")]
    LegacyUnavailable(&'static str),
}

impl CliError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Clap(_) => "argument_error",
            Self::CurrentDir(_) => "current_dir_error",
            Self::Graph(_) => "graph_error",
            Self::Backend(_) => "backend_error",
            Self::Selector(_) => "selector_parse_error",
            Self::Json(_) => "json_error",
            Self::Stdout(_) => "stdout_error",
            Self::LegacyProcess { .. } => "legacy_process_error",
            Self::LegacyTool { .. } => "legacy_tool_error",
            Self::LegacyUnavailable(_) => "legacy_unavailable",
        }
    }

    pub fn details(&self) -> Option<&str> {
        match self {
            Self::Graph(GraphError::InvalidData { reason, .. }) => Some(reason.as_str()),
            Self::Graph(GraphError::Io { reason, .. }) => Some(reason.as_str()),
            Self::Graph(GraphError::Sqlite { reason, .. }) => Some(reason.as_str()),
            Self::Graph(GraphError::Unimplemented) => None,
            Self::LegacyTool { stderr, .. } => Some(stderr.as_str()),
            _ => None,
        }
    }
}
