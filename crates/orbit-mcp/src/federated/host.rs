//! The federated mux host: one MCP surface over many configured destinations.

use std::str::FromStr;
use std::sync::Arc;

use orbit_common::OrbitError;
use orbit_types::tool::{
    McpToolDefinition, McpToolScope, ToolSchema, ToolSessionContext, mcp_advertised_tool_name,
};
use orbit_types::workspace::WorkspaceStatus;
use serde_json::{Map, Value, json};

use super::config::{Destination, HostQualifiedSelector};
use super::descriptor::WorkspaceDescriptor;
use super::probe::{DestinationProbe, DestinationSnapshot};

/// Federated discovery stays session-unbound and is answered by the mux.
///
/// Every other advertised tool is delivered to the destination encoded in the
/// caller's host-qualified selector.
pub const FEDERATED_WORKSPACE_LIST_TOOL: &str = "orbit.workspace.list";

/// An MCP host that aggregates operator-configured destinations.
pub struct FederatedMcpHost {
    destinations: Vec<Destination>,
    probe: Arc<dyn DestinationProbe>,
}

impl FederatedMcpHost {
    pub fn new(destinations: Vec<Destination>, probe: Arc<dyn DestinationProbe>) -> Self {
        Self {
            destinations,
            probe,
        }
    }

    /// The federated list, in configured order.
    ///
    /// `machine_id` is on each descriptor, not the envelope: one response now
    /// spans many machines, so a single envelope-level identity would be a lie
    /// about all but one of them.
    fn list_workspaces(&self) -> Value {
        json!({ "workspaces": self.probe_all_destinations() })
    }

    /// Probe every destination concurrently, so the list costs the slowest
    /// destination rather than the sum of all of them.
    fn probe_all_destinations(&self) -> Vec<WorkspaceDescriptor> {
        std::thread::scope(|scope| {
            let probes = self
                .destinations
                .iter()
                .map(|destination| {
                    (
                        destination,
                        scope.spawn(move || self.describe_destination(destination)),
                    )
                })
                .collect::<Vec<_>>();
            probes
                .into_iter()
                .flat_map(|(destination, probe)| {
                    probe.join().unwrap_or_else(|_| {
                        tracing::warn!(
                            machine_id = %destination.machine_id,
                            "federated destination probe panicked",
                        );
                        vec![WorkspaceDescriptor::unreachable(destination)]
                    })
                })
                .collect()
        })
    }

    /// One destination's rows. Never empty: a configured destination the caller
    /// cannot see is worse than one it can see is down.
    fn describe_destination(&self, destination: &Destination) -> Vec<WorkspaceDescriptor> {
        let snapshot = match self.probe.probe(destination) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::warn!(
                    machine_id = %destination.machine_id,
                    %error,
                    "federated destination did not answer its live probe",
                );
                return vec![WorkspaceDescriptor::unreachable(destination)];
            }
        };
        if let Err(error) = confirm_pinned_identity(destination, &snapshot) {
            // A destination answering under a different identity is not a new
            // error class: whatever answered, the configured machine did not.
            tracing::warn!(
                machine_id = %destination.machine_id,
                %error,
                "federated destination answered under a different machine_id",
            );
            return vec![WorkspaceDescriptor::unreachable(destination)];
        }
        if snapshot.workspaces.is_empty() {
            return vec![WorkspaceDescriptor::workspaceless(destination)];
        }
        snapshot
            .workspaces
            .into_iter()
            .map(|workspace| WorkspaceDescriptor::reachable(destination, workspace))
            .collect()
    }

    /// Deliver a workspace-scoped call to the destination encoded in the selector.
    ///
    /// Classification uses a live session, not the last list. Fail-closed
    /// precedence: unknown selector, then unreachable, stale, unhealthy,
    /// tool-not-on-this-host, then the destination's own refusal.
    fn route_workspace_call(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        let token = workspace_selector(&input, &session_context).ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "tool '{name}' requires a workspace selector; pass `workspace` in the tool call \
                 or MCP initialize metadata"
            ))
        })?;
        let parsed = HostQualifiedSelector::from_str(token)?;
        let destination = self
            .destinations
            .iter()
            .find(|destination| destination.machine_id == parsed.machine_id())
            .ok_or_else(|| OrbitError::UnknownSelector(token.to_string()))?;

        let mut session = self
            .probe
            .open_route(destination)
            .map_err(|error| delivery_unreachable(destination, error))?;
        let snapshot = session
            .snapshot()
            .map_err(|error| delivery_unreachable(destination, error))?;
        if let Err(error) = confirm_pinned_identity(destination, &snapshot) {
            tracing::warn!(
                machine_id = %destination.machine_id,
                %error,
                "federated destination answered under a different machine_id",
            );
            return Err(error);
        }
        let Some(workspace) = snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.id == parsed.workspace_id())
        else {
            return Err(OrbitError::StaleRoute(token.to_string()));
        };
        if workspace.status == WorkspaceStatus::Invalid {
            return Err(OrbitError::UnhealthyCheckout(token.to_string()));
        }

        let advertised = session
            .advertised_tools()
            .map_err(|error| delivery_unreachable(destination, error))?;
        if !tool_on_surface(&advertised, name) {
            return Err(OrbitError::ToolNotOnThisHost(format!(
                "'{name}' is not advertised on '{}'",
                destination.machine_id
            )));
        }

        tracing::info!(
            machine_id = %destination.machine_id,
            workspace_id = %parsed.workspace_id(),
            tool = name,
            "federated mux delivering tool call"
        );
        session.call_tool(name, destination_arguments(input, parsed.workspace_id()))
    }
}

impl crate::McpHost for FederatedMcpHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        let mut definitions = crate::canonical_mcp_tool_definitions()
            .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;
        for definition in &mut definitions {
            if definition.schema.name == FEDERATED_WORKSPACE_LIST_TOOL {
                *definition = federated_workspace_list_definition();
            }
        }
        Ok(definitions)
    }

    fn call_tool(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        if name == FEDERATED_WORKSPACE_LIST_TOOL {
            return Ok(self.list_workspaces());
        }
        self.route_workspace_call(name, input, session_context)
    }
}

/// The federated list definition.
///
/// [`McpToolScope::Global`] is what makes it session-unbound: it takes no
/// workspace selector, and an announced session workspace is neither an input
/// nor a filter. The description is deliberately not v1's — this is a new
/// response shape, not a compatible extension of the machine-local list.
fn federated_workspace_list_definition() -> McpToolDefinition {
    McpToolDefinition::new(
        ToolSchema {
            name: FEDERATED_WORKSPACE_LIST_TOOL.to_string(),
            description: "List every configured destination's workspaces as live descriptors, \
                          including destinations that are unreachable right now. Copy a row's \
                          `selector` to address that workspace; do not parse or construct it."
                .to_string(),
            parameters: Vec::new(),
            builtin: true,
        },
        McpToolScope::Global,
    )
}

/// The operator's config pin is the identity of record; a live answer only
/// confirms it.
fn confirm_pinned_identity(
    destination: &Destination,
    snapshot: &DestinationSnapshot,
) -> Result<(), OrbitError> {
    if snapshot.machine_id == destination.machine_id {
        return Ok(());
    }
    Err(OrbitError::UnreachableDestination(format!(
        "'{}' is configured as machine '{}' but answered as '{}'",
        destination.ssh, destination.machine_id, snapshot.machine_id
    )))
}

/// The selector the call itself passed, else the session's announced one.
fn workspace_selector<'a>(input: &'a Value, context: &'a ToolSessionContext) -> Option<&'a str> {
    input
        .get("workspace")
        .and_then(Value::as_str)
        .or(context.workspace.as_deref())
        .map(str::trim)
        .filter(|selector| !selector.is_empty())
}

/// v1 destinations address a local `ws_*`, not the host-qualified token.
fn destination_arguments(input: Value, workspace_id: &str) -> Value {
    let mut object = match input {
        Value::Object(object) => object,
        _ => Map::new(),
    };
    object.insert(
        "workspace".to_string(),
        Value::String(workspace_id.to_string()),
    );
    Value::Object(object)
}

fn tool_on_surface(advertised: &[String], name: &str) -> bool {
    let wire = mcp_advertised_tool_name(name);
    advertised
        .iter()
        .any(|tool| tool == name || *tool == wire || mcp_advertised_tool_name(tool) == wire)
}

/// Connect, snapshot, and tools/list failures are delivery misses: capability
/// and stale are undecidable without the host.
fn delivery_unreachable(destination: &Destination, error: OrbitError) -> OrbitError {
    match error {
        OrbitError::UnreachableDestination(_) => error,
        other => OrbitError::UnreachableDestination(format!("{}: {other}", destination.machine_id)),
    }
}
