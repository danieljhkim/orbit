use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use orbit_common::OrbitError;
use orbit_types::identity::{validate_machine_id, validate_registry_identifier};
use orbit_types::tool::mcp_advertised_tool_name;
use orbit_types::workspace::{Workspace, WorkspaceCheckout, WorkspaceCheckoutRole};
use serde::Deserialize;

pub const DESTINATIONS_FILE: &str = "mcp-destinations.toml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostQualifiedSelector {
    machine_id: String,
    workspace_id: String,
}

impl HostQualifiedSelector {
    pub fn machine_id(&self) -> &str {
        &self.machine_id
    }

    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }
}

impl FromStr for HostQualifiedSelector {
    type Err = OrbitError;

    fn from_str(token: &str) -> Result<Self, Self::Err> {
        let (machine_id, workspace_id) = token
            .split_once('/')
            .ok_or_else(|| unknown_selector(token))?;
        validate_machine_id(machine_id).map_err(|_| unknown_selector(token))?;
        validate_workspace_id(workspace_id).map_err(|_| unknown_selector(token))?;
        Ok(Self {
            machine_id: machine_id.to_string(),
            workspace_id: workspace_id.to_string(),
        })
    }
}

impl fmt::Display for HostQualifiedSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.machine_id, self.workspace_id)
    }
}

fn validate_workspace_id(workspace_id: &str) -> Result<(), ()> {
    validate_registry_identifier("workspace_id", workspace_id).map_err(|_| ())?;
    let suffix = workspace_id.strip_prefix("ws_").ok_or(())?;
    if suffix.is_empty()
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(());
    }
    Ok(())
}

fn unknown_selector(token: &str) -> OrbitError {
    OrbitError::UnknownSelector(token.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Destination {
    pub ssh: String,
    pub machine_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DestinationsFile {
    pub destinations: Vec<Destination>,
}

pub fn destinations_path(global_orbit_root: &Path) -> PathBuf {
    global_orbit_root.join(DESTINATIONS_FILE)
}

pub fn load_destinations(path: &Path) -> Result<DestinationsFile, OrbitError> {
    let contents = std::fs::read_to_string(path).map_err(|error| {
        OrbitError::Io(format!(
            "failed to read federated MCP destinations '{}': {error}",
            path.display()
        ))
    })?;
    let destinations: DestinationsFile = toml::from_str(&contents).map_err(|error| {
        OrbitError::InvalidInput(format!(
            "invalid federated MCP destinations '{}': {error}",
            path.display()
        ))
    })?;
    validate_destinations(&destinations, path)?;
    Ok(destinations)
}

fn validate_destinations(destinations: &DestinationsFile, path: &Path) -> Result<(), OrbitError> {
    let mut machine_ids = HashSet::with_capacity(destinations.destinations.len());
    for destination in &destinations.destinations {
        if !machine_ids.insert(destination.machine_id.as_str()) {
            return Err(OrbitError::AmbiguousDestination(format!(
                "machine_id '{}' appears more than once in '{}'",
                destination.machine_id,
                path.display()
            )));
        }
    }
    for destination in &destinations.destinations {
        validate_machine_id(&destination.machine_id).map_err(|error| {
            OrbitError::InvalidInput(format!(
                "federated MCP destinations '{}' has invalid machine_id '{}': {error}",
                path.display(),
                destination.machine_id
            ))
        })?;
        if destination.ssh.trim().is_empty() {
            return Err(OrbitError::InvalidInput(format!(
                "federated MCP destinations '{}' has a blank ssh target for '{}'",
                path.display(),
                destination.machine_id
            )));
        }
    }
    Ok(())
}

/// Capability class of an advertised MCP tool.
///
/// Assigned by what the tool does, so a new tool inherits its class from its
/// behavior rather than from a per-tool registry field [ORB-11012].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpToolClass {
    /// Discovery and list tools. Never subject to `capability_refused`.
    Unclassified,
    /// Task issuance and the coordination store, whose authority is the
    /// declared control-plane owner rather than the destination host.
    ControlPlane,
    /// Runs, logs, and scheduler state, which the destination host owns.
    Execute,
}

impl McpToolClass {
    pub fn as_str(self) -> &'static str {
        match self {
            McpToolClass::Unclassified => "unclassified",
            McpToolClass::ControlPlane => "control_plane",
            McpToolClass::Execute => "execute",
        }
    }
}

/// Classify one advertised MCP tool by behavior.
///
/// Accepts either the canonical (`orbit.task.add`) or advertised
/// (`orbit_task_add`) spelling. Task reads are `control_plane` because the
/// coordination store is owner-authoritative. A name this host does not
/// advertise is unclassified here; routing rejects it earlier with
/// `tool_not_on_this_host`, which precedes `capability_refused`.
pub fn mcp_tool_class(tool_name: &str) -> McpToolClass {
    match mcp_advertised_tool_name(tool_name).as_str() {
        "orbit_task_add"
        | "orbit_task_update"
        | "orbit_task_start"
        | "orbit_task_approve"
        | "orbit_task_list"
        | "orbit_task_show"
        | "orbit_task_artifact_put"
        | "orbit_friction_add"
        | "orbit_friction_list"
        | "orbit_friction_update"
        | "orbit_auto_task_list"
        | "orbit_auto_task_mint"
        | "orbit_search"
        | "orbit_workflow_ship" => McpToolClass::ControlPlane,
        "orbit_command_exec"
        | "orbit_workflow_run_list"
        | "orbit_workflow_run_show"
        | "orbit_workflow_run_resume"
        | "orbit_session_log_append"
        | "orbit_session_log_list"
        | "orbit_session_log_resolve" => McpToolClass::Execute,
        _ => McpToolClass::Unclassified,
    }
}

/// Capability classes a destination advertises for one workspace checkout.
///
/// Advertisement is a hint that may lag; destination Core refusal stays the
/// correctness boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityClasses {
    control_plane: bool,
    execute: bool,
}

impl CapabilityClasses {
    pub fn new(control_plane: bool, execute: bool) -> Self {
        Self {
            control_plane,
            execute,
        }
    }

    /// Advertisement helper: the classes this local checkout holds.
    ///
    /// An owner checkout is the workspace's control-plane authority and also
    /// runs work locally today. A replica is an execution binding only. A
    /// workspace whose logical record omits `owner_machine_id` predates host
    /// identity and cannot advertise `control_plane` at all. A missing checkout
    /// role is the legacy standalone shape that registry validation
    /// canonicalizes to owner.
    pub fn for_checkout(workspace: &Workspace, checkout: &WorkspaceCheckout) -> Self {
        let is_replica = checkout.role == Some(WorkspaceCheckoutRole::Replica);
        Self {
            control_plane: !is_replica && workspace.owner_machine_id.is_some(),
            execute: true,
        }
    }

    pub fn holds(self, class: McpToolClass) -> bool {
        match class {
            McpToolClass::Unclassified => true,
            McpToolClass::ControlPlane => self.control_plane,
            McpToolClass::Execute => self.execute,
        }
    }
}

/// Destination-side gate: refuse a tool whose class this checkout does not hold.
///
/// Unclassified discovery tools always pass. The refusal carries the class so a
/// caller can distinguish it from a malformed call or an operator capability
/// grant.
pub fn ensure_tool_class_held(tool_name: &str, held: CapabilityClasses) -> Result<(), OrbitError> {
    let class = mcp_tool_class(tool_name);
    if held.holds(class) {
        return Ok(());
    }
    Err(OrbitError::CapabilityRefused(format!(
        "this checkout does not hold the '{}' capability class required by '{tool_name}'",
        class.as_str()
    )))
}

#[cfg(test)]
#[path = "tests/federated.rs"]
mod tests;
