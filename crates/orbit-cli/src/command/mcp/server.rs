//! Local composition for the authoritative MCP server process.

use std::path::PathBuf;
use std::sync::Arc;

use orbit_cmd::registry_runtime::RegisteredRuntimeFactory;
use orbit_common::types::{
    McpToolDefinition, McpToolScope, NotFoundKind, OrbitError, ToolSessionContext,
    WorkspaceCheckoutRole,
};
use orbit_core::OrbitRuntime;
use orbit_core::command::tool::ToolEntryPoint;
use orbit_core::runtime::resolve_global_root;
use orbit_mcp::McpHost;
use serde_json::Value;

use super::crew::crew_discovery;

pub(super) fn serve_mcp_stdio(remote_caller_machine_id: Option<String>) -> Result<(), OrbitError> {
    let global_root = resolve_global_root()?;
    let identity = orbit_mcp::mcp_server_identity(&global_root, remote_caller_machine_id)?;
    let host = Arc::new(ServerMcpHost::new(
        global_root,
        identity.process_machine_id,
        identity.process_host_id,
    ));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| OrbitError::Execution(format!("tokio runtime: {error}")))?;
    runtime.block_on(orbit_mcp::serve_stdio_with_context(
        host,
        identity.session_context,
    ))
}

/// One MCP server bound to the executing machine.
struct ServerMcpHost {
    global_root: PathBuf,
    process_machine_id: String,
    process_host_id: String,
}

impl ServerMcpHost {
    fn new(global_root: PathBuf, process_machine_id: String, process_host_id: String) -> Self {
        Self {
            global_root,
            process_machine_id,
            process_host_id,
        }
    }

    fn definition(&self, name: &str) -> Result<McpToolDefinition, OrbitError> {
        orbit_mcp::canonical_mcp_tool_definitions()
            .map_err(|error| OrbitError::InvalidInput(error.to_string()))?
            .into_iter()
            .find(|definition| definition.schema.name == name)
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Tool, name.to_string()))
    }

    fn workspace_selector<'a>(
        input: &'a Value,
        context: &'a ToolSessionContext,
    ) -> Option<&'a str> {
        input
            .get("workspace")
            .and_then(Value::as_str)
            .or(context.workspace.as_deref())
            .map(str::trim)
            .filter(|selector| !selector.is_empty())
    }

    fn workspace_required(&self, name: &str) -> OrbitError {
        OrbitError::InvalidInput(format!(
            "tool '{name}' requires a workspace selector; pass `workspace` in the tool call or MCP initialize metadata"
        ))
    }

    fn list_workspaces(&self) -> Result<Value, OrbitError> {
        let registry_path =
            orbit_registry::workspace_registry::registry_path_for(&self.global_root);
        let registry = orbit_registry::workspace_registry::load_registry_from(&registry_path)?;
        orbit_mcp::execute_discovery_tool(
            "orbit.workspace.list",
            &registry,
            &self.process_machine_id,
        )
    }

    fn call_workspace_tool(
        &self,
        name: &str,
        mut input: Value,
        mut context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        let selector = Self::workspace_selector(&input, &context)
            .ok_or_else(|| self.workspace_required(name))?;
        let selected =
            RegisteredRuntimeFactory::resolve_workspace_selector(&self.global_root, selector)?;
        orbit_cmd::registry_runtime::sync_runtime_task_prefix(&self.global_root)?;
        let binding = orbit_core::runtime::workspace_runtime_binding(
            &selected.workspace,
            &selected.checkout,
        )?;
        let replica_owner = (selected.checkout.role == Some(WorkspaceCheckoutRole::Replica))
            .then(|| selected.checkout.owner_machine_id.clone())
            .flatten();
        let runtime = OrbitRuntime::from_roots_with_binding(
            &self.global_root,
            &selected.checkout.orbit_dir,
            binding,
        )?
        .with_coordination_write_owner(replica_owner);
        let repo_root = selected.checkout.repo_root.to_string_lossy().into_owned();

        context.workspace_id = Some(selected.workspace.id.clone());
        context.workspace = Some(repo_root.clone());
        context.process_machine_id = Some(self.process_machine_id.clone());
        context.process_host_id = Some(self.process_host_id.clone());

        if let Some(object) = input.as_object_mut()
            && object.contains_key("workspace")
        {
            object.insert("workspace".to_string(), Value::String(repo_root));
        }

        if name == "orbit.crew.list" {
            let global_root = self.global_root.clone();
            let checkout_orbit_dir = selected.checkout.orbit_dir.clone();
            let workspace_id = selected.workspace.id.clone();
            let owner_machine_id = selected.workspace.owner_machine_id.clone();
            return runtime
                .execute_in_process_tool_dispatch(
                    name,
                    input,
                    ToolEntryPoint::Mcp,
                    context,
                    move |_| {
                        serde_json::to_value(crew_discovery(
                            &global_root,
                            &checkout_orbit_dir,
                            &workspace_id,
                            owner_machine_id,
                        )?)
                        .map_err(|error| {
                            OrbitError::Execution(format!("serialize crew discovery: {error}"))
                        })
                    },
                )
                .map(|outcome| outcome.value);
        }

        execute_core_tool(&runtime, name, input, context)
    }
}

impl McpHost for ServerMcpHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        orbit_mcp::canonical_mcp_tool_definitions()
            .map_err(|error| OrbitError::InvalidInput(error.to_string()))
    }

    fn call_tool(
        &self,
        name: &str,
        input: Value,
        context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        let definition = self.definition(name)?;
        if definition.policy.scope() == McpToolScope::Global {
            return match name {
                "orbit.workspace.list" => self.list_workspaces(),
                _ => Err(OrbitError::not_found(NotFoundKind::Tool, name.to_string())),
            };
        }
        self.call_workspace_tool(name, input, context)
    }
}

fn execute_core_tool(
    runtime: &OrbitRuntime,
    name: &str,
    input: Value,
    context: ToolSessionContext,
) -> Result<Value, OrbitError> {
    runtime
        .execute_tool_command_dispatch_with_session_context(
            name,
            input,
            None,
            None,
            ToolEntryPoint::Mcp,
            context,
        )
        .map(|outcome| outcome.value)
}
