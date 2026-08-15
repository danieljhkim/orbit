use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::OrbitError;

/// Maximum encoded length for a stable registry identifier.
pub const REGISTRY_IDENTIFIER_MAX_BYTES: usize = 128;
/// Namespace prefix for generated machine identifiers.
pub const MACHINE_ID_PREFIX: &str = "hm_";

/// Validate a path-free, normalized identifier stored in public registry
/// records. Transport targets and filesystem paths are never identities.
pub fn validate_registry_identifier(field: &str, value: &str) -> Result<(), OrbitError> {
    if value.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "{field} must not be empty"
        )));
    }
    if value.trim() != value {
        return Err(OrbitError::InvalidInput(format!(
            "{field} must not contain leading or trailing whitespace"
        )));
    }
    if value.len() > REGISTRY_IDENTIFIER_MAX_BYTES {
        return Err(OrbitError::InvalidInput(format!(
            "{field} must not exceed {REGISTRY_IDENTIFIER_MAX_BYTES} bytes"
        )));
    }
    if value.chars().any(char::is_control) || value.contains(['/', '\\']) {
        return Err(OrbitError::InvalidInput(format!(
            "{field} must be a logical registry identifier, not a path"
        )));
    }
    Ok(())
}

/// Validate the stable machine key used in host and workspace role records.
pub fn validate_machine_id(machine_id: &str) -> Result<(), OrbitError> {
    validate_registry_identifier("machine_id", machine_id)?;
    let Some(suffix) = machine_id.strip_prefix(MACHINE_ID_PREFIX) else {
        return Err(OrbitError::InvalidInput(
            "machine_id must use the canonical 'hm_' namespace".to_string(),
        ));
    };
    if suffix.is_empty()
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(OrbitError::InvalidInput(
            "machine_id must contain 'hm_' followed by ASCII letters, digits, '_' or '-'"
                .to_string(),
        ));
    }
    Ok(())
}

/// Validate a human-readable host name stored by the hub registry.
pub fn validate_host_id(host_id: &str) -> Result<(), OrbitError> {
    validate_registry_identifier("host_id", host_id)
}

/// Durable lifecycle state of a registered Orbit host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostStatus {
    Active,
    Retired,
}

impl HostStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Retired => "retired",
        }
    }
}

impl fmt::Display for HostStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for HostStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "active" => Ok(Self::Active),
            "retired" => Ok(Self::Retired),
            other => Err(format!("unknown host status '{other}'")),
        }
    }
}

/// Stable fields supplied when a machine registers with the hub.
///
/// Registration timestamps and liveness are store-owned. Repeating this
/// declaration is idempotent only while all three fields remain compatible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostRegistration {
    pub machine_id: String,
    pub host_id: String,
    #[serde(default)]
    pub labels: BTreeSet<String>,
}

/// Durable host-registry projection. It intentionally contains no transport,
/// credential, filesystem, or workspace-coordination fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostRecord {
    pub machine_id: String,
    pub host_id: String,
    pub labels: BTreeSet<String>,
    pub status: HostStatus,
    pub registered_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub retired_at: Option<DateTime<Utc>>,
    /// Updated only by explicit registry operations. Audit traffic is never a
    /// liveness source.
    pub last_seen_at: Option<DateTime<Utc>>,
}

/// One permanent historical name for a registered machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostAlias {
    pub alias_host_id: String,
    pub machine_id: String,
    pub created_at: DateTime<Utc>,
    pub warning: String,
}

/// Typed result of resolving a human-authored host name.
///
/// The `Retired` variant is returned for both a retired host's current name
/// and its old aliases. `Collision` is retained as an explicit fail-closed
/// outcome even though the current schema and typed mutations prevent it; it
/// keeps callers safe if externally modified or legacy data is inconsistent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostNameResolution {
    Active {
        host: HostRecord,
    },
    Alias {
        host: HostRecord,
        alias: HostAlias,
    },
    Retired {
        host: HostRecord,
        alias: Option<HostAlias>,
    },
    Unknown {
        host_id: String,
    },
    Collision {
        host_id: String,
        machine_ids: Vec<String>,
    },
}
