//! Support for `orbit mcp` and `orbit serve mcp`.
//!
//! `orbit mcp init/remove` manages local client integration for Claude Code
//! and Codex. `orbit serve mcp` serves the Orbit tool surface over MCP so
//! external clients can discover and invoke Orbit operations with typed JSON
//! schemas.

mod setup;

use std::path::Path;
use std::sync::Arc;

use clap::{Args, Subcommand};
use orbit_common::types::ToolSchema;
use orbit_core::{OrbitError, OrbitRuntime};
use orbit_mcp::McpHost;
use serde_json::Value;

use crate::command::Execute;

pub(crate) use setup::init_auto_for_workspace;
pub use setup::{InitArgs, RemoveArgs};

pub(crate) const ORBIT_MCP_SERVER_ID: &str = "orbit";

pub(crate) const TASK_TOOL_NAMES: &[&str] = &[
    "orbit.task.add",
    "orbit.task.approve",
    "orbit.task.delete",
    "orbit.task.lint",
    "orbit.task.list",
    "orbit.task.locks",
    "orbit.task.locks.release",
    "orbit.task.locks.reserve",
    "orbit.task.reject",
    "orbit.task.review_thread.add",
    "orbit.task.review_thread.list",
    "orbit.task.review_thread.reply",
    "orbit.task.review_thread.resolve",
    "orbit.task.show",
    "orbit.task.start",
    "orbit.task.update",
];

pub(crate) const GRAPH_READ_TOOL_NAMES: &[&str] = &[
    "orbit.graph.callers",
    "orbit.graph.deps",
    "orbit.graph.implementors",
    "orbit.graph.overview",
    "orbit.graph.pack",
    "orbit.graph.refs",
    "orbit.graph.search",
    "orbit.graph.show",
];

pub(crate) const GRAPH_WRITE_TOOL_NAMES: &[&str] = &[
    "orbit.graph.add",
    "orbit.graph.delete",
    "orbit.graph.move",
    "orbit.graph.write",
];

pub(crate) fn safe_mcp_tool_names(allow_write: bool) -> Vec<&'static str> {
    let mut names = Vec::with_capacity(
        TASK_TOOL_NAMES.len()
            + GRAPH_READ_TOOL_NAMES.len()
            + if allow_write {
                GRAPH_WRITE_TOOL_NAMES.len()
            } else {
                0
            },
    );
    names.extend_from_slice(TASK_TOOL_NAMES);
    names.extend_from_slice(GRAPH_READ_TOOL_NAMES);
    if allow_write {
        names.extend_from_slice(GRAPH_WRITE_TOOL_NAMES);
    }
    names
}

pub(crate) fn is_mcp_tool_exposed(name: &str, allow_write: bool) -> bool {
    TASK_TOOL_NAMES.contains(&name)
        || GRAPH_READ_TOOL_NAMES.contains(&name)
        || (allow_write && GRAPH_WRITE_TOOL_NAMES.contains(&name))
}

fn ensure_mcp_tool_exposed(name: &str, allow_write: bool) -> Result<(), OrbitError> {
    if is_mcp_tool_exposed(name, allow_write) {
        Ok(())
    } else {
        Err(OrbitError::ToolNotFound(name.to_string()))
    }
}

#[derive(Args)]
#[command(
    about = "Manage Orbit MCP client integrations",
    arg_required_else_help = true,
    subcommand_required = true
)]
pub struct McpCommand {
    #[command(subcommand)]
    pub command: McpSubcommand,
}

impl McpCommand {
    pub fn execute_without_runtime(self, root_override: Option<&Path>) -> Result<(), OrbitError> {
        self.command.execute_without_runtime(root_override)
    }
}

impl Execute for McpCommand {
    fn execute(self, _runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.execute_without_runtime(None)
    }
}

#[derive(Subcommand)]
pub enum McpSubcommand {
    /// Initialize MCP client integration for the current workspace
    Init(InitArgs),
    /// Remove MCP client integration for the current workspace
    Remove(RemoveArgs),
}

impl McpSubcommand {
    pub fn execute_without_runtime(self, root_override: Option<&Path>) -> Result<(), OrbitError> {
        match self {
            Self::Init(args) => args.execute_without_runtime(root_override),
            Self::Remove(args) => args.execute_without_runtime(root_override),
        }
    }
}

impl Execute for McpSubcommand {
    fn execute(self, _runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.execute_without_runtime(None)
    }
}

#[derive(Args)]
#[command(about = "Serve the Orbit tool registry over Model Context Protocol")]
pub struct ServeArgs {
    /// Expose experimental graph write tools in addition to the safe default surface.
    #[arg(long)]
    pub allow_write: bool,
}

impl Execute for ServeArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let host: Arc<dyn McpHost> = Arc::new(RuntimeMcpHost {
            runtime: runtime.clone(),
            allow_write: self.allow_write,
        });

        let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| OrbitError::Execution(format!("tokio runtime: {e}")))?;

        tokio_runtime.block_on(orbit_mcp::serve_stdio(host))
    }
}

/// [`McpHost`] impl that forwards every MCP operation through the full
/// [`OrbitRuntime`] pipeline.
///
/// Listing is sourced from [`OrbitRuntime::list_tools`], which already filters
/// disabled tools and merges external (non-builtin) entries. Execution is
/// routed through [`OrbitRuntime::execute_tool_command`], which applies the
/// same policy evaluation, workspace sandboxing, and audit event emission as
/// the CLI `orbit tool run` path.
struct RuntimeMcpHost {
    runtime: OrbitRuntime,
    allow_write: bool,
}

impl McpHost for RuntimeMcpHost {
    fn list_tool_schemas(&self) -> Vec<ToolSchema> {
        let tools = self.runtime.list_tools().unwrap_or_default();
        tools
            .into_iter()
            .filter(|tool| tool.enabled && is_mcp_tool_exposed(&tool.name, self.allow_write))
            .map(|tool| ToolSchema {
                name: tool.name,
                description: tool.description,
                parameters: tool.parameters,
                builtin: tool.builtin,
            })
            .collect()
    }

    fn call_tool(&self, name: &str, input: Value) -> Result<Value, OrbitError> {
        ensure_mcp_tool_exposed(name, self.allow_write)?;
        self.runtime.execute_tool_command(name, input, None, None)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use orbit_core::OrbitRuntime;

    use super::{
        GRAPH_READ_TOOL_NAMES, GRAPH_WRITE_TOOL_NAMES, TASK_TOOL_NAMES, is_mcp_tool_exposed,
        safe_mcp_tool_names,
    };

    #[test]
    fn safe_surface_matches_runtime_graph_and_task_tools() {
        let runtime = OrbitRuntime::in_memory().expect("build test runtime");
        let names: BTreeSet<String> = runtime
            .list_tools()
            .expect("list tools")
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        let safe_names: BTreeSet<&str> = safe_mcp_tool_names(false).into_iter().collect();

        for name in TASK_TOOL_NAMES {
            assert!(names.contains(*name), "missing runtime task tool: {name}");
            assert!(is_mcp_tool_exposed(name, false));
        }

        for name in names.iter().filter(|name| name.starts_with("orbit.task.")) {
            assert!(
                safe_names.contains(name.as_str()),
                "runtime task tool missing from safe MCP surface: {name}"
            );
        }

        for name in GRAPH_READ_TOOL_NAMES {
            assert!(
                names.contains(*name),
                "missing runtime graph read tool: {name}"
            );
            assert!(is_mcp_tool_exposed(name, false));
        }

        for name in GRAPH_WRITE_TOOL_NAMES {
            assert!(
                names.contains(*name),
                "missing runtime graph write tool: {name}"
            );
            assert!(!is_mcp_tool_exposed(name, false));
            assert!(is_mcp_tool_exposed(name, true));
        }

        assert!(!is_mcp_tool_exposed("orbit.state.get", false));
        assert!(!is_mcp_tool_exposed("demo.hello", false));
    }

    #[test]
    fn allow_write_surface_adds_graph_write_tools() {
        let safe = safe_mcp_tool_names(false);
        let writable = safe_mcp_tool_names(true);

        assert!(writable.len() > safe.len());
        for name in GRAPH_WRITE_TOOL_NAMES {
            assert!(!safe.contains(name));
            assert!(writable.contains(name));
        }
    }
}
