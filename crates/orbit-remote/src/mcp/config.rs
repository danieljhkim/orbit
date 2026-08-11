//! Trusted machine-global MCP topology configuration [ORB-10268, ORB-10727].
//!
//! This file is deliberately separate from Orbit's layered workspace config:
//! `mcp.toml` is read only from the already-resolved machine-global Orbit root
//! and names zero or more owner machines. It never accepts credentials,
//! commands, workspace targets, or environment overlays.
//!
//! ORB-10268 froze the on-disk boundary as one optional singular `[hub]` table;
//! ADR-0355 restates it as zero or more `[[owner]]` entries in the same file.
//! The rest of the boundary is unchanged: the whole document and every entry
//! reject unknown fields, `transport` is exactly `ssh`, aliases are
//! argument-safe OpenSSH host aliases, and each allowed-capability list is
//! non-empty, duplicate-free, and typed `agent|operator`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use orbit_common::types::{McpCapability, OrbitError, validate_machine_id};
use serde::Deserialize;

pub(super) const MCP_TOML_FILE: &str = "mcp.toml";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct TrustedMcpConfig {
    /// Owner routes keyed by the target owner `machine_id`. `mcp.toml` cannot
    /// redefine ownership, so this is a lookup table, not an authority.
    pub(super) owners: BTreeMap<String, TrustedOwnerRoute>,
}

/// One validated owner route. `transport` is not carried here: the typed
/// [`OwnerTransport`] parse is the whole check, and the SSH command Orbit
/// constructs is fixed, so a stored copy could only re-derive the single value
/// the parser admits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TrustedOwnerRoute {
    pub(super) machine_id: String,
    pub(super) host: String,
    pub(super) allowed_capabilities: BTreeSet<McpCapability>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum OwnerTransport {
    Ssh,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMcpConfig {
    #[serde(default)]
    owner: Vec<RawOwnerRoute>,
    /// Legacy singular hub table, accepted only so the loader can fail with a
    /// migration message instead of a bare unknown-field error. See
    /// [`legacy_hub_table_error`].
    #[serde(default)]
    hub: Option<toml::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOwnerRoute {
    machine_id: String,
    /// Frozen at exactly `ssh`; deserialization is the validation.
    #[expect(
        dead_code,
        reason = "the typed parse rejects any other transport, which is the entire contract"
    )]
    transport: OwnerTransport,
    host: String,
    // Keep the vector until validation so duplicate declarations cannot be
    // erased by set conversion.
    allowed_capabilities: Vec<McpCapability>,
}

impl TrustedMcpConfig {
    fn from_raw(raw: RawMcpConfig, path: &Path) -> Result<Self, OrbitError> {
        if raw.hub.is_some() {
            return Err(legacy_hub_table_error(path));
        }
        let mut owners = BTreeMap::new();
        for entry in raw.owner {
            let route = TrustedOwnerRoute::from_raw(entry, path)?;
            if owners.contains_key(&route.machine_id) {
                return Err(OrbitError::InvalidInput(format!(
                    "trusted MCP config '{}' declares more than one [[owner]] entry for machine_id '{}'; each owner machine is named exactly once",
                    path.display(),
                    route.machine_id
                )));
            }
            owners.insert(route.machine_id.clone(), route);
        }
        Ok(Self { owners })
    }

    /// Every configured owner route, in stable `machine_id` order.
    pub(super) fn routes(&self) -> impl Iterator<Item = &TrustedOwnerRoute> {
        self.owners.values()
    }
}

impl TrustedOwnerRoute {
    fn from_raw(raw: RawOwnerRoute, path: &Path) -> Result<Self, OrbitError> {
        validate_machine_id(&raw.machine_id).map_err(|error| {
            OrbitError::InvalidInput(format!(
                "trusted MCP config '{}' has invalid owner machine_id: {error}",
                path.display()
            ))
        })?;
        validate_ssh_alias(&raw.host).map_err(|message| {
            OrbitError::InvalidInput(format!(
                "trusted MCP config '{}' has invalid owner host alias: {message}",
                path.display()
            ))
        })?;
        if raw.allowed_capabilities.is_empty() {
            return Err(OrbitError::InvalidInput(format!(
                "trusted MCP config '{}' owner.allowed_capabilities must not be empty",
                path.display()
            )));
        }
        let mut allowed_capabilities = BTreeSet::new();
        for capability in raw.allowed_capabilities {
            if !capability.is_bridge_v1() {
                return Err(OrbitError::InvalidInput(format!(
                    "trusted MCP config '{}' grants owner capability '{capability}', which the v1 bridge withdrew (ADR-0358); use agent and/or operator",
                    path.display()
                )));
            }
            if !allowed_capabilities.insert(capability) {
                return Err(OrbitError::InvalidInput(format!(
                    "trusted MCP config '{}' repeats owner capability '{capability}'",
                    path.display()
                )));
            }
        }
        Ok(Self {
            machine_id: raw.machine_id,
            host: raw.host,
            allowed_capabilities,
        })
    }
}

/// Fail closed on a legacy `[hub]` table rather than auto-migrating it.
///
/// ORB-10727 decision: a `[hub]` entry named a *machine-level* coordination
/// host, while `[[owner]]` names the machine that owns a given workspace. Those
/// are not the same claim — the machine in `[hub]` need not own any of this
/// machine's workspaces — so silently rewriting one into the other could route
/// coordination calls to a non-owner. Rewriting the file is a one-line human
/// edit; guessing the routing target is not something the loader may do.
fn legacy_hub_table_error(path: &Path) -> OrbitError {
    OrbitError::InvalidInput(format!(
        "trusted MCP config '{}' still declares the withdrawn singular [hub] table. \
         ADR-0355 replaced the machine-level hub with per-owner routes, and a hub target is \
         not automatically the owner of any workspace, so this is not migrated for you. \
         Replace the table with one [[owner]] entry per owner machine, keeping the same \
         fields:\n\n\
         [[owner]]\n\
         machine_id = \"<owner machine_id from workspaces.json>\"\n\
         transport = \"ssh\"\n\
         host = \"<OpenSSH host alias>\"\n\
         allowed_capabilities = [\"agent\"]\n",
        path.display()
    ))
}

/// Load only `<global_root>/mcp.toml`. A missing file is the valid default for
/// a machine that owns every workspace it uses; a call that needs an absent
/// route is refused later, by owner placement preflight.
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
