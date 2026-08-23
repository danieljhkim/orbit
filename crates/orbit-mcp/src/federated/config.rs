//! Operator-configured federated destinations and the host-qualified selector.

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use orbit_common::OrbitError;
use orbit_types::identity::{validate_machine_id, validate_registry_identifier};
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
