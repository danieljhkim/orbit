//! The federated mux host: one MCP surface over many configured destinations.

use std::sync::Arc;

use orbit_common::OrbitError;
use orbit_types::tool::{McpToolDefinition, McpToolScope, ToolSchema, ToolSessionContext};
use serde_json::{Value, json};

use super::config::Destination;
use super::descriptor::WorkspaceDescriptor;
use super::probe::{DestinationProbe, DestinationSnapshot};

/// The only tool the mux advertises today.
///
/// Routing has not landed, so advertising the rest of the canonical surface
/// would promise delivery the mux cannot perform. Callers that already chose a
/// host keep the v1 paths.
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
}

impl crate::McpHost for FederatedMcpHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        Ok(vec![federated_workspace_list_definition()])
    }

    fn call_tool(
        &self,
        name: &str,
        _input: Value,
        _session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        if name == FEDERATED_WORKSPACE_LIST_TOOL {
            return Ok(self.list_workspaces());
        }
        Err(OrbitError::ToolNotOnThisHost(format!(
            "'{name}' is not on the federated surface; only '{FEDERATED_WORKSPACE_LIST_TOOL}' is \
             routed today"
        )))
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
