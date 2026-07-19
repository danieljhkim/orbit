use std::collections::HashMap;
use std::sync::Mutex;

use orbit_common::types::{
    LearningInjectionState, McpCapability, McpToolDefinition, McpToolPlacement, McpToolPolicy,
    OrbitError, ToolSchema, ToolSessionContext,
};
use orbit_mcp::McpHost;
use orbit_mcp::OrbitToolServer;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientInfo, Meta};
use rmcp::service::{RoleClient, RunningService};
use serde_json::{Value, json};
use tokio::io::duplex;

use super::super::learning::LearningSidecarHost as LearningSidecarHostContract;

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
    schemas
        .into_iter()
        .map(|schema| {
            let policy = McpToolPolicy::new(McpToolPlacement::LocalDerived, [McpCapability::Agent])
                .expect("test MCP policy has one static capability");
            McpToolDefinition::new(schema, policy)
                .map_err(|error| OrbitError::InvalidInput(error.to_string()))
        })
        .collect()
}

pub(super) fn request_with_args(name: &str, args: Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name.replace('.', "_")).with_arguments(
        args.as_object()
            .expect("object args")
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    )
}

pub(super) struct LearningSidecarHost {
    response: Value,
    search_by_path: HashMap<String, Vec<Value>>,
    calls: Mutex<Vec<String>>,
    session_states: Mutex<HashMap<String, LearningInjectionState>>,
}

impl LearningSidecarHost {
    pub(super) fn new(response: Value, search_by_path: HashMap<String, Vec<Value>>) -> Self {
        Self {
            response,
            search_by_path,
            calls: Mutex::new(Vec::new()),
            session_states: Mutex::new(HashMap::new()),
        }
    }
}

impl McpHost for LearningSidecarHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        test_mcp_definitions([
            tool_schema("orbit.task.show"),
            tool_schema("orbit.learning.list"),
        ])
    }

    fn call_tool(
        &self,
        name: &str,
        input: Value,
        _session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        self.calls
            .lock()
            .expect("calls lock")
            .push(name.to_string());
        if name == "orbit.learning.list" {
            let path = input
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return Ok(Value::Array(
                self.search_by_path.get(path).cloned().unwrap_or_default(),
            ));
        }
        Ok(self.response.clone())
    }
}

impl LearningSidecarHostContract for LearningSidecarHost {
    fn get_session_learning_state(
        &self,
        session_id: &str,
    ) -> Result<Option<LearningInjectionState>, OrbitError> {
        Ok(self
            .session_states
            .lock()
            .expect("session states lock")
            .get(session_id)
            .cloned())
    }

    fn upsert_session_learning_state(
        &self,
        session_id: &str,
        state: &LearningInjectionState,
    ) -> Result<(), OrbitError> {
        self.session_states
            .lock()
            .expect("session states lock")
            .insert(session_id.to_string(), state.clone());
        Ok(())
    }
}

pub(super) struct WireServer {
    client: RunningService<RoleClient, ClientInfo>,
    server: tokio::task::JoinHandle<()>,
}

impl WireServer {
    pub(super) async fn new(server: OrbitToolServer, workspace: Option<&str>) -> Self {
        let (client_io, server_io) = duplex(1 << 20);
        let (client_read, client_write) = tokio::io::split(client_io);
        let (server_read, server_write) = tokio::io::split(server_io);
        let server = tokio::spawn(async move {
            let service = server
                .serve((server_read, server_write))
                .await
                .expect("serve in-memory MCP fixture");
            service.waiting().await.expect("wait for MCP fixture");
        });
        let mut client_info = ClientInfo::default();
        if let Some(workspace) = workspace {
            client_info.meta = Some(Meta(
                json!({ "orbit": { "workspace": workspace } })
                    .as_object()
                    .expect("initialize metadata object")
                    .clone(),
            ));
        }
        let client = client_info
            .serve((client_read, client_write))
            .await
            .expect("connect in-memory MCP fixture");
        Self { client, server }
    }

    pub(super) async fn call(&self, name: &str, args: Value) -> CallToolResult {
        self.client
            .peer()
            .call_tool(request_with_args(name, args))
            .await
            .expect("MCP fixture call")
    }
}

impl Drop for WireServer {
    fn drop(&mut self) {
        self.server.abort();
    }
}
