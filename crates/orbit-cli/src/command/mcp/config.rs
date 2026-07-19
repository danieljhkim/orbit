//! Trusted machine-global MCP topology configuration [ORB-10268].
//!
//! This file is deliberately separate from Orbit's layered workspace config:
//! `mcp.toml` is read only from the already-resolved machine-global Orbit root
//! and contains one optional, singular hub route. It never accepts credentials,
//! commands, owner routes, workspace targets, or environment overlays.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use orbit_common::types::{McpCapability, OrbitError, validate_machine_id};
use orbit_remote::{HostIdentity, HostMode};
use serde::Deserialize;

pub(super) const MCP_TOML_FILE: &str = "mcp.toml";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct TrustedMcpConfig {
    pub(super) hub: Option<TrustedHubConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TrustedHubConfig {
    pub(super) machine_id: String,
    pub(super) transport: HubTransport,
    pub(super) host: String,
    pub(super) allowed_capabilities: BTreeSet<McpCapability>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum HubTransport {
    Ssh,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMcpConfig {
    #[serde(default)]
    hub: Option<RawHubConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHubConfig {
    machine_id: String,
    transport: HubTransport,
    host: String,
    // Keep the vector until validation so duplicate declarations cannot be
    // erased by set conversion.
    allowed_capabilities: Vec<McpCapability>,
}

impl TrustedMcpConfig {
    fn from_raw(raw: RawMcpConfig, path: &Path) -> Result<Self, OrbitError> {
        let hub = raw
            .hub
            .map(|hub| TrustedHubConfig::from_raw(hub, path))
            .transpose()?;
        Ok(Self { hub })
    }

    /// Resolve the one permitted spoke-to-hub route and exact scalar grant.
    /// No route is ever returned for a hub or standalone machine.
    pub(super) fn spoke_route(
        &self,
        identity: &HostIdentity,
        requested: Option<McpCapability>,
    ) -> Result<(&TrustedHubConfig, McpCapability), OrbitError> {
        if identity.mode != HostMode::Spoke {
            return Err(OrbitError::InvalidInput(format!(
                "an MCP hub transport route requires spoke mode, not '{}'",
                identity.mode
            )));
        }
        let hub = self.hub.as_ref().ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "spoke '{}' ({}) has no trusted hub route in machine-global {MCP_TOML_FILE}",
                identity.host_id, identity.machine_id
            ))
        })?;
        if hub.machine_id == identity.machine_id {
            return Err(OrbitError::InvalidInput(format!(
                "trusted hub machine_id '{}' matches this spoke; refusing a self route",
                hub.machine_id
            )));
        }
        let effective = requested.unwrap_or(McpCapability::Agent);
        if !hub.allowed_capabilities.contains(&effective) {
            return Err(OrbitError::InvalidInput(format!(
                "MCP capability '{effective}' is not granted by machine-global {MCP_TOML_FILE} for hub machine_id '{}'",
                hub.machine_id
            )));
        }
        Ok((hub, effective))
    }

    /// Resolve one deterministic scalar capability for the private bootstrap
    /// registration request. Registration requires no operator privilege and
    /// never expands the configured set: agent is preferred for ordinary
    /// workstations, then operator, then runner-only pollers.
    pub(super) fn spoke_registration_route(
        &self,
        identity: &HostIdentity,
    ) -> Result<(&TrustedHubConfig, McpCapability), OrbitError> {
        let hub = self.hub.as_ref().ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "spoke '{}' ({}) has no trusted hub route in machine-global {MCP_TOML_FILE}",
                identity.host_id, identity.machine_id
            ))
        })?;
        let capability = [
            McpCapability::Agent,
            McpCapability::Operator,
            McpCapability::Runner,
        ]
        .into_iter()
        .find(|capability| hub.allowed_capabilities.contains(capability))
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "trusted hub route in machine-global {MCP_TOML_FILE} grants no registration capability"
            ))
        })?;
        self.spoke_route(identity, Some(capability))
    }
}

impl TrustedHubConfig {
    fn from_raw(raw: RawHubConfig, path: &Path) -> Result<Self, OrbitError> {
        validate_machine_id(&raw.machine_id).map_err(|error| {
            OrbitError::InvalidInput(format!(
                "trusted MCP config '{}' has invalid hub machine_id: {error}",
                path.display()
            ))
        })?;
        validate_ssh_alias(&raw.host).map_err(|message| {
            OrbitError::InvalidInput(format!(
                "trusted MCP config '{}' has invalid hub host alias: {message}",
                path.display()
            ))
        })?;
        if raw.allowed_capabilities.is_empty() {
            return Err(OrbitError::InvalidInput(format!(
                "trusted MCP config '{}' hub.allowed_capabilities must not be empty",
                path.display()
            )));
        }
        let mut allowed_capabilities = BTreeSet::new();
        for capability in raw.allowed_capabilities {
            if !allowed_capabilities.insert(capability) {
                return Err(OrbitError::InvalidInput(format!(
                    "trusted MCP config '{}' repeats hub capability '{capability}'",
                    path.display()
                )));
            }
        }
        Ok(Self {
            machine_id: raw.machine_id,
            transport: raw.transport,
            host: raw.host,
            allowed_capabilities,
        })
    }
}

/// Load only `<global_root>/mcp.toml`. A missing file is the valid hub-local
/// default; a spoke that needs a route fails later in [`TrustedMcpConfig::spoke_route`].
pub(super) fn load_trusted_mcp_config(global_root: &Path) -> Result<TrustedMcpConfig, OrbitError> {
    let path = mcp_toml_path(global_root);
    if !path.exists() {
        return Ok(TrustedMcpConfig::default());
    }
    let raw = std::fs::read_to_string(&path).map_err(|error| {
        OrbitError::Io(format!(
            "failed to read trusted MCP config '{}': {error}",
            path.display()
        ))
    })?;
    let parsed: RawMcpConfig = toml::from_str(&raw).map_err(|error| {
        OrbitError::InvalidInput(format!(
            "invalid trusted MCP config '{}': {error}",
            path.display()
        ))
    })?;
    TrustedMcpConfig::from_raw(parsed, &path)
}

pub(super) fn mcp_toml_path(global_root: &Path) -> PathBuf {
    global_root.join(MCP_TOML_FILE)
}

fn validate_ssh_alias(alias: &str) -> Result<(), &'static str> {
    if alias.is_empty() {
        return Err("must not be empty");
    }
    if alias.len() > 253 {
        return Err("must not exceed 253 bytes");
    }
    let mut bytes = alias.bytes();
    if !bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
    {
        return Err("must begin with an ASCII letter or digit");
    }
    if !bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')) {
        return Err("may contain only ASCII letters, digits, '.', '_' or '-'");
    }
    Ok(())
}
