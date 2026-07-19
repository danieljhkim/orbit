//! Versioned owner-published execution facts for multi-host coordination.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::activity_job::{Backend, Provider};
use super::registry_snapshot::RegistryProfileV1;
use super::{Crew, OrbitError, validate_machine_id};

pub const EXECUTION_PROFILE_SCHEMA_VERSION: u32 = 1;
pub const EXECUTION_CONFIG_DIGEST_DOMAIN: &[u8] = b"orbit.execution-profile.config.v1\0";

/// Schema version of the sanitized [`CrewDiscoveryV1`] projection returned by
/// `orbit.crew.list`. Bumped only for a backward-incompatible projection change.
pub const CREW_DISCOVERY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceOwnership {
    pub workspace_id: String,
    pub owner_machine_id: String,
    pub bound_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Private, host-keyed checkout declaration. `root` is deliberately absent
/// from every sanitized/public projection below.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostWorkspacePresence {
    pub machine_id: String,
    pub workspace_id: String,
    pub root: PathBuf,
    pub last_verified: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspacePresenceDeclaration {
    pub workspace_id: String,
    pub root: PathBuf,
    pub last_verified: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionFreshness {
    Missing,
    Current,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SanitizedWorkspacePresence {
    pub workspace_id: String,
    pub machine_id: String,
    pub owner_machine_id: Option<String>,
    pub freshness: ProjectionFreshness,
    pub last_verified: Option<DateTime<Utc>>,
    pub age_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionProfileCrewV1 {
    pub name: String,
    pub provider: String,
    pub model: String,
    pub backend: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
}

impl ExecutionProfileCrewV1 {
    pub fn from_crew(crew: &Crew, auto_backend: Backend) -> Result<Self, OrbitError> {
        let name = required_trimmed("crew name", &crew.name)?;
        let provider = Provider::parse(&crew.assignment.provider).map_err(|error| {
            OrbitError::InvalidInput(format!("crew '{name}' has invalid provider: {error}"))
        })?;
        let model = required_trimmed("crew model", &crew.assignment.model)?;
        let configured_backend = Backend::parse(&crew.assignment.backend).ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "crew '{name}' has unknown backend '{}'",
                crew.assignment.backend
            ))
        })?;
        let backend = match configured_backend {
            Backend::Auto => auto_backend,
            concrete => concrete,
        };
        if backend == Backend::Auto {
            return Err(OrbitError::InvalidInput(format!(
                "crew '{name}' did not resolve backend:auto to a concrete backend"
            )));
        }
        match backend {
            Backend::Http if !provider.has_http_transport() => {
                return Err(OrbitError::InvalidInput(format!(
                    "crew '{name}' selects provider '{}' without an HTTP transport",
                    provider.as_str()
                )));
            }
            Backend::Cli if !provider.has_cli_runtime() => {
                return Err(OrbitError::InvalidInput(format!(
                    "crew '{name}' selects provider '{}' without a CLI runtime",
                    provider.as_str()
                )));
            }
            Backend::Http | Backend::Cli => {}
            Backend::Auto => unreachable!("auto rejected above"),
        }

        let description = crew
            .description
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let mut tags = crew
            .tags
            .iter()
            .map(|tag| tag.trim())
            .filter(|tag| !tag.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        tags.sort();
        tags.dedup();

        Ok(Self {
            name,
            provider: provider.as_str().to_string(),
            model,
            backend: backend.as_str().to_string(),
            description,
            tags,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionProfileShipV1 {
    pub mode: String,
    pub base_branch: String,
    pub ship_closure_digest: String,
}

/// Frozen owner payload. Hub-owned generation/receipt metadata belongs only
/// to [`StoredExecutionProfile`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionProfileV1 {
    pub schema_version: u32,
    pub workspace_id: String,
    pub owner_machine_id: String,
    pub observed_at: DateTime<Utc>,
    pub config_digest: String,
    pub default_crew: String,
    pub crews: Vec<ExecutionProfileCrewV1>,
    pub ship: ExecutionProfileShipV1,
}

impl ExecutionProfileV1 {
    pub fn validate(&self) -> Result<(), OrbitError> {
        if self.schema_version != EXECUTION_PROFILE_SCHEMA_VERSION {
            return Err(OrbitError::InvalidInput(format!(
                "unsupported execution profile schema_version {}",
                self.schema_version
            )));
        }
        validate_logical_id("workspace_id", &self.workspace_id)?;
        validate_machine_id(&self.owner_machine_id)?;
        let default_crew = required_trimmed("default_crew", &self.default_crew)?;
        if default_crew != self.default_crew {
            return Err(OrbitError::InvalidInput(
                "default_crew must be normalized without surrounding whitespace".to_string(),
            ));
        }
        if self.crews.is_empty() {
            return Err(OrbitError::InvalidInput(
                "execution profile must contain at least one crew".to_string(),
            ));
        }
        let mut previous: Option<&str> = None;
        let mut has_default = false;
        for crew in &self.crews {
            validate_profile_crew(crew)?;
            if previous.is_some_and(|name| name >= crew.name.as_str()) {
                return Err(OrbitError::InvalidInput(
                    "execution profile crews must have unique names sorted ascending".to_string(),
                ));
            }
            previous = Some(&crew.name);
            has_default |= crew.name == self.default_crew;
        }
        if !has_default {
            return Err(OrbitError::InvalidInput(format!(
                "default_crew '{}' does not name an execution profile crew",
                self.default_crew
            )));
        }
        if !matches!(self.ship.mode.as_str(), "pr" | "local") {
            return Err(OrbitError::InvalidInput(format!(
                "execution profile ship mode '{}' is not pr or local",
                self.ship.mode
            )));
        }
        let base_branch = required_trimmed("ship base_branch", &self.ship.base_branch)?;
        if base_branch != self.ship.base_branch {
            return Err(OrbitError::InvalidInput(
                "ship base_branch must be normalized without surrounding whitespace".to_string(),
            ));
        }
        validate_sha256("config_digest", &self.config_digest)?;
        validate_sha256("ship_closure_digest", &self.ship.ship_closure_digest)?;
        let expected = self.compute_config_digest()?;
        if self.config_digest != expected {
            return Err(OrbitError::InvalidInput(format!(
                "execution profile config_digest mismatch: expected {expected}"
            )));
        }
        Ok(())
    }

    pub fn compute_config_digest(&self) -> Result<String, OrbitError> {
        #[derive(Serialize)]
        struct ConfigDigestPayload<'a> {
            schema_version: u32,
            default_crew: &'a str,
            crews: &'a [ExecutionProfileCrewV1],
            ship_mode: &'a str,
            ship_base_branch: &'a str,
        }

        let payload = ConfigDigestPayload {
            schema_version: self.schema_version,
            default_crew: &self.default_crew,
            crews: &self.crews,
            ship_mode: &self.ship.mode,
            ship_base_branch: &self.ship.base_branch,
        };
        let canonical = serde_json::to_vec(&payload)
            .map_err(|error| OrbitError::Store(format!("serialize config digest: {error}")))?;
        let mut hasher = Sha256::new();
        hasher.update(EXECUTION_CONFIG_DIGEST_DOMAIN);
        hasher.update(canonical);
        Ok(format!("{:x}", hasher.finalize()))
    }

    pub fn semantic_eq(&self, other: &Self) -> bool {
        self.schema_version == other.schema_version
            && self.workspace_id == other.workspace_id
            && self.owner_machine_id == other.owner_machine_id
            && self.config_digest == other.config_digest
            && self.default_crew == other.default_crew
            && self.crews == other.crews
            && self.ship == other.ship
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredExecutionProfile {
    pub profile: ExecutionProfileV1,
    pub generation: u64,
    pub received_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SanitizedExecutionProfile {
    pub workspace_id: String,
    pub owner_machine_id: Option<String>,
    pub freshness: ProjectionFreshness,
    pub generation: Option<u64>,
    pub observed_at: Option<DateTime<Utc>>,
    pub received_at: Option<DateTime<Utc>>,
    pub age_seconds: Option<u64>,
    pub profile: Option<ExecutionProfileV1>,
}

/// Sanitized `orbit.crew.list` projection: stable workspace/owner identity, the
/// shared [`RegistryProfileV1`] freshness/generation envelope (also used by
/// `orbit.workspace.list`), the default crew, and the sorted effective crew
/// entries.
///
/// It deliberately carries no `config_digest`, ship closure, presence root,
/// checkout path, raw profile payload, environment name/value, secret,
/// credential, token, SSH material, command fragment, or repository content:
/// only the allowlisted name/provider/model/backend/description/tags crew
/// projection appears. A stale record still returns its crews, but they are
/// bound to the stale `profile` envelope generation/freshness and never carry a
/// dispatch-eligible marker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrewDiscoveryV1 {
    pub schema_version: u32,
    pub workspace_id: String,
    pub owner_machine_id: Option<String>,
    pub profile: RegistryProfileV1,
    pub default_crew: Option<String>,
    pub crews: Vec<ExecutionProfileCrewV1>,
}

/// One reusable, immutable typed validation result shared by crew discovery and
/// explicit task-crew validation. It captures the exact owner profile lineage
/// that a validated dispatch is bound to — stored profile, resolved crew,
/// hub-owned generation, config digest, and ship-closure digest — without
/// persisting any run lineage or leasing state (those belong to H3/I1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedCrewProfile {
    pub workspace_id: String,
    pub owner_machine_id: String,
    pub generation: u64,
    pub config_digest: String,
    pub ship_closure_digest: String,
    pub resolved_crew: ExecutionProfileCrewV1,
    pub profile: ExecutionProfileV1,
}

fn validate_profile_crew(crew: &ExecutionProfileCrewV1) -> Result<(), OrbitError> {
    let name = required_trimmed("crew name", &crew.name)?;
    if name != crew.name {
        return Err(OrbitError::InvalidInput(format!(
            "crew '{}' name is not normalized",
            crew.name
        )));
    }
    let provider = Provider::parse(&crew.provider).map_err(|error| {
        OrbitError::InvalidInput(format!(
            "crew '{}' has invalid provider: {error}",
            crew.name
        ))
    })?;
    if provider.as_str() != crew.provider {
        return Err(OrbitError::InvalidInput(format!(
            "crew '{}' provider '{}' is not canonical",
            crew.name, crew.provider
        )));
    }
    let model = required_trimmed("crew model", &crew.model)?;
    if model != crew.model {
        return Err(OrbitError::InvalidInput(format!(
            "crew '{}' model is not normalized",
            crew.name
        )));
    }
    let backend = Backend::parse(&crew.backend).ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "crew '{}' has unknown backend '{}'",
            crew.name, crew.backend
        ))
    })?;
    if backend == Backend::Auto || backend.as_str() != crew.backend {
        return Err(OrbitError::InvalidInput(format!(
            "crew '{}' backend '{}' is not concrete and canonical",
            crew.name, crew.backend
        )));
    }
    match backend {
        Backend::Http if !provider.has_http_transport() => {
            return Err(OrbitError::InvalidInput(format!(
                "crew '{}' selects provider '{}' without an HTTP transport",
                crew.name, crew.provider
            )));
        }
        Backend::Cli if !provider.has_cli_runtime() => {
            return Err(OrbitError::InvalidInput(format!(
                "crew '{}' selects provider '{}' without a CLI runtime",
                crew.name, crew.provider
            )));
        }
        Backend::Http | Backend::Cli => {}
        Backend::Auto => unreachable!("auto rejected above"),
    }
    if crew
        .description
        .as_deref()
        .is_some_and(|value| value.is_empty() || value.trim() != value)
    {
        return Err(OrbitError::InvalidInput(format!(
            "crew '{}' description is not normalized",
            crew.name
        )));
    }
    let mut previous: Option<&str> = None;
    for tag in &crew.tags {
        if tag.is_empty() || tag.trim() != tag {
            return Err(OrbitError::InvalidInput(format!(
                "crew '{}' contains an empty or non-normalized tag",
                crew.name
            )));
        }
        if previous.is_some_and(|value| value >= tag.as_str()) {
            return Err(OrbitError::InvalidInput(format!(
                "crew '{}' tags must be sorted and deduplicated",
                crew.name
            )));
        }
        previous = Some(tag);
    }
    Ok(())
}

fn required_trimmed(field: &str, value: &str) -> Result<String, OrbitError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "{field} must not be empty"
        )));
    }
    Ok(trimmed.to_string())
}

fn validate_logical_id(field: &str, value: &str) -> Result<(), OrbitError> {
    let normalized = required_trimmed(field, value)?;
    if normalized != value || value.chars().any(char::is_control) || value.contains(['/', '\\']) {
        return Err(OrbitError::InvalidInput(format!(
            "{field} must be a normalized logical identifier, not a path"
        )));
    }
    Ok(())
}

fn validate_sha256(field: &str, value: &str) -> Result<(), OrbitError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(OrbitError::InvalidInput(format!(
            "{field} must be 64 lowercase hexadecimal characters"
        )));
    }
    Ok(())
}
