use std::sync::Mutex as StdMutex;

use orbit_common::types::{
    McpToolDefinition, McpToolScope, OrbitError, ToolParam, ToolSchema, ToolSessionContext,
};
use rmcp::model::CallToolRequestParams;
use serde_json::{Value, json};

use super::name_map::sanitize_tool_name;

pub(super) fn param_with_type(name: &str, param_type: &str) -> ToolParam {
    ToolParam {
        name: name.to_string(),
        description: String::new(),
        param_type: param_type.to_string(),
        required: false,
    }
}

pub(super) fn param(name: &str) -> ToolParam {
    param_with_type(name, "string")
}

pub(super) fn tool_schema(name: &str) -> ToolSchema {
    ToolSchema {
        name: name.to_string(),
        description: String::new(),
        parameters: Vec::new(),
        builtin: true,
    }
}

pub(super) fn test_mcp_definitions(
    schemas: impl IntoIterator<Item = ToolSchema>,
) -> Result<Vec<McpToolDefinition>, OrbitError> {
    Ok(schemas
        .into_iter()
        .map(|schema| McpToolDefinition::new(schema, McpToolScope::WorkspaceRequired))
        .collect())
}

pub(super) fn request_with_args(name: &str, args: Value) -> CallToolRequestParams {
    CallToolRequestParams::new(sanitize_tool_name(name)).with_arguments(
        args.as_object()
            .expect("object args")
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    )
}

pub(super) struct StubHost {
    pub(super) schemas: Vec<ToolSchema>,
}

impl crate::McpHost for StubHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        test_mcp_definitions(self.schemas.clone())
    }

    fn call_tool(
        &self,
        _name: &str,
        _input: Value,
        _session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        Ok(Value::Null)
    }
}

pub(super) struct EchoArrayHost {
    pub(super) schemas: Vec<ToolSchema>,
}

impl crate::McpHost for EchoArrayHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        test_mcp_definitions(self.schemas.clone())
    }

    fn call_tool(
        &self,
        name: &str,
        _input: Value,
        _session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        Ok(json!([{ "tool": name }]))
    }
}

#[derive(Default)]
pub(super) struct SessionContextHost {
    calls: StdMutex<Vec<(String, Value, ToolSessionContext)>>,
}

impl SessionContextHost {
    pub(super) fn calls(&self) -> Vec<(String, Value, ToolSessionContext)> {
        self.calls.lock().expect("calls lock").clone()
    }
}

impl crate::McpHost for SessionContextHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        test_mcp_definitions(vec![
            tool_schema("orbit.task.list"),
            tool_schema("orbit.task.add"),
        ])
    }

    fn call_tool(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        let effective_workspace = input
            .get("workspace")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| session_context.workspace.clone());
        self.calls.lock().expect("calls lock").push((
            name.to_string(),
            input.clone(),
            session_context.clone(),
        ));
        Ok(json!({
            "tool": name,
            "effective_workspace": effective_workspace,
        }))
    }
}
