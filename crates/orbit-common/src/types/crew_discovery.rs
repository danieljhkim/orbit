//! Sanitized crew discovery types shared by MCP hosts and clients.

use serde::{Deserialize, Serialize};

use super::activity_job::{Backend, Provider};
use super::{Crew, OrbitError};

/// Schema version of the [`CrewDiscoveryV1`] projection returned by
/// `orbit.crew.list`.
pub const CREW_DISCOVERY_SCHEMA_VERSION: u32 = 1;

/// One effective crew entry exposed by [`CrewDiscoveryV1`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrewDiscoveryEntryV1 {
    pub name: String,
    pub provider: String,
    pub model: String,
    pub backend: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
}

impl CrewDiscoveryEntryV1 {
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

/// Sanitized `orbit.crew.list` projection for one selected workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrewDiscoveryV1 {
    pub schema_version: u32,
    pub workspace_id: String,
    pub owner_machine_id: Option<String>,
    pub default_crew: Option<String>,
    pub crews: Vec<CrewDiscoveryEntryV1>,
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
