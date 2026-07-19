//! Frozen private wire contract for spoke self-registration.
//!
//! This is deliberately not a tool schema. The only wire method using these
//! DTOs is the connector-private `orbit/private/register-spoke/v1` request.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{
    ExecutionProfileV1, HostRecord, HostRegistration, HostStatus, OrbitError,
    REGISTRY_SNAPSHOT_SCHEMA_VERSION, RegistrySnapshotV1, WorkspacePresenceDeclaration,
    validate_host_id, validate_machine_id, validate_registry_identifier,
};

/// Exact rmcp custom-request method used by the trusted spoke connector.
pub const SPOKE_REGISTRATION_METHOD_V1: &str = "orbit/private/register-spoke/v1";

/// Schema version shared by the private request and result envelopes.
pub const SPOKE_REGISTRATION_SCHEMA_VERSION: u32 = 1;

/// One owner profile publication with the generation observed by the spoke.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpokeExecutionProfilePublicationV1 {
    pub expected_generation: u64,
    pub profile: ExecutionProfileV1,
}

/// Strict connector-built registration payload.
///
/// Identity is copied from validated machine-local `host.toml`. Presence and
/// profiles are copied from typed local workspace-registry/runtime builders.
/// Presence roots are the sole path-bearing spoke-to-hub exception and never
/// appear in [`SpokeRegistrationResultV1`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpokeRegistrationRequestV1 {
    pub schema_version: u32,
    pub identity: HostRegistration,
    #[serde(default)]
    pub presence: Vec<WorkspacePresenceDeclaration>,
    #[serde(default)]
    pub profiles: Vec<SpokeExecutionProfilePublicationV1>,
}

impl SpokeRegistrationRequestV1 {
    pub fn validate(&self) -> Result<(), OrbitError> {
        if self.schema_version != SPOKE_REGISTRATION_SCHEMA_VERSION {
            return Err(OrbitError::InvalidInput(format!(
                "unsupported spoke registration schema_version {}; expected {}",
                self.schema_version, SPOKE_REGISTRATION_SCHEMA_VERSION
            )));
        }
        validate_machine_id(&self.identity.machine_id)?;
        validate_host_id(&self.identity.host_id)?;
        for label in &self.identity.labels {
            validate_registry_identifier("host label", label)?;
        }

        let mut presence_ids = BTreeSet::new();
        for declaration in &self.presence {
            validate_registry_identifier("workspace_id", &declaration.workspace_id)?;
            if !presence_ids.insert(declaration.workspace_id.as_str()) {
                return Err(OrbitError::InvalidInput(format!(
                    "spoke registration presence repeats workspace_id '{}'",
                    declaration.workspace_id
                )));
            }
            if !declaration.root.is_absolute() {
                return Err(OrbitError::InvalidInput(format!(
                    "spoke registration presence root for workspace_id '{}' must be absolute",
                    declaration.workspace_id
                )));
            }
            let root = declaration.root.to_str().ok_or_else(|| {
                OrbitError::InvalidInput(format!(
                    "spoke registration presence root for workspace_id '{}' must be valid UTF-8",
                    declaration.workspace_id
                ))
            })?;
            if root.chars().any(char::is_control) {
                return Err(OrbitError::InvalidInput(format!(
                    "spoke registration presence root for workspace_id '{}' must not contain control characters",
                    declaration.workspace_id
                )));
            }
        }

        let mut profile_ids = BTreeSet::new();
        for publication in &self.profiles {
            publication.profile.validate()?;
            if publication.profile.owner_machine_id != self.identity.machine_id {
                return Err(OrbitError::InvalidInput(format!(
                    "execution profile owner_machine_id '{}' does not match registering machine_id '{}'",
                    publication.profile.owner_machine_id, self.identity.machine_id
                )));
            }
            if !profile_ids.insert(publication.profile.workspace_id.as_str()) {
                return Err(OrbitError::InvalidInput(format!(
                    "spoke registration profiles repeat workspace_id '{}'",
                    publication.profile.workspace_id
                )));
            }
        }
        Ok(())
    }
}

/// Hub-side stages whose commits cannot be rolled back across the link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpokeRegistrationStageV1 {
    Registry,
    Presence,
    Profiles,
    Snapshot,
}

/// Definitive hub-side failure returned inside the typed custom result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpokeRegistrationFailureV1 {
    pub code: String,
    pub message: String,
}

/// Sanitized result of the staged hub registration flow.
///
/// Partial success is an ordinary typed result: `last_committed_stage` names
/// durable hub state and `failure` names the repair reason. Only a complete
/// result carries the sanitized snapshot eligible for local cache refresh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpokeRegistrationResultV1 {
    pub schema_version: u32,
    pub complete: bool,
    pub last_committed_stage: Option<SpokeRegistrationStageV1>,
    pub host: Option<HostRecord>,
    #[serde(default)]
    pub presence_workspace_ids: Vec<String>,
    #[serde(default)]
    pub profile_workspace_ids: Vec<String>,
    pub snapshot: Option<RegistrySnapshotV1>,
    pub failure: Option<SpokeRegistrationFailureV1>,
}

impl SpokeRegistrationResultV1 {
    pub fn rejected(error: &OrbitError) -> Self {
        Self::failed(
            None,
            None,
            Vec::new(),
            Vec::new(),
            registration_error_code(error),
            error.to_string(),
        )
    }

    pub fn failed(
        last_committed_stage: Option<SpokeRegistrationStageV1>,
        host: Option<HostRecord>,
        presence_workspace_ids: Vec<String>,
        profile_workspace_ids: Vec<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: SPOKE_REGISTRATION_SCHEMA_VERSION,
            complete: false,
            last_committed_stage,
            host,
            presence_workspace_ids,
            profile_workspace_ids,
            snapshot: None,
            failure: Some(SpokeRegistrationFailureV1 {
                code: code.into(),
                message: message.into(),
            }),
        }
    }

    pub fn completed(
        host: HostRecord,
        presence_workspace_ids: Vec<String>,
        profile_workspace_ids: Vec<String>,
        snapshot: RegistrySnapshotV1,
    ) -> Self {
        Self {
            schema_version: SPOKE_REGISTRATION_SCHEMA_VERSION,
            complete: true,
            last_committed_stage: Some(SpokeRegistrationStageV1::Snapshot),
            host: Some(host),
            presence_workspace_ids,
            profile_workspace_ids,
            snapshot: Some(snapshot),
            failure: None,
        }
    }

    pub fn validate(&self) -> Result<(), OrbitError> {
        if self.schema_version != SPOKE_REGISTRATION_SCHEMA_VERSION {
            return Err(OrbitError::InvalidInput(format!(
                "unsupported spoke registration result schema_version {}; expected {}",
                self.schema_version, SPOKE_REGISTRATION_SCHEMA_VERSION
            )));
        }
        validate_workspace_ids("presence_workspace_ids", &self.presence_workspace_ids)?;
        validate_workspace_ids("profile_workspace_ids", &self.profile_workspace_ids)?;
        if let Some(host) = &self.host {
            validate_machine_id(&host.machine_id)?;
            validate_host_id(&host.host_id)?;
        }
        if let Some(failure) = &self.failure
            && (failure.code.trim().is_empty() || failure.message.trim().is_empty())
        {
            return Err(OrbitError::InvalidInput(
                "invalid spoke registration result: failure code and message must be non-empty"
                    .to_string(),
            ));
        }

        match (
            self.complete,
            self.last_committed_stage,
            &self.host,
            &self.snapshot,
            &self.failure,
        ) {
            (
                true,
                Some(SpokeRegistrationStageV1::Snapshot),
                Some(host),
                Some(snapshot),
                None,
            ) if host.status == HostStatus::Active
                && snapshot.schema_version == REGISTRY_SNAPSHOT_SCHEMA_VERSION
                && snapshot.hub_machine_id.is_some()
                && snapshot.hosts.iter().any(|entry| {
                    entry.machine_id == host.machine_id && entry.host_id == host.host_id
                }) => Ok(()),
            (false, None, None, None, Some(_))
                if self.presence_workspace_ids.is_empty()
                    && self.profile_workspace_ids.is_empty() => Ok(()),
            (
                false,
                Some(SpokeRegistrationStageV1::Registry),
                Some(_),
                None,
                Some(_),
            ) if self.presence_workspace_ids.is_empty()
                && self.profile_workspace_ids.is_empty() => Ok(()),
            (
                false,
                Some(SpokeRegistrationStageV1::Presence),
                Some(_),
                None,
                Some(_),
            ) if self.profile_workspace_ids.is_empty() => Ok(()),
            (
                false,
                Some(SpokeRegistrationStageV1::Profiles),
                Some(_),
                None,
                Some(_),
            ) if !self.profile_workspace_ids.is_empty() => Ok(()),
            _ => Err(OrbitError::InvalidInput(
                "invalid spoke registration result: complete results require a snapshot and no failure; partial results require a failure and no snapshot"
                    .to_string(),
            )),
        }
    }
}

fn validate_workspace_ids(field: &str, workspace_ids: &[String]) -> Result<(), OrbitError> {
    let mut unique = BTreeSet::new();
    for workspace_id in workspace_ids {
        validate_registry_identifier("workspace_id", workspace_id)?;
        if !unique.insert(workspace_id) {
            return Err(OrbitError::InvalidInput(format!(
                "spoke registration result {field} repeats workspace_id '{workspace_id}'"
            )));
        }
    }
    Ok(())
}

fn registration_error_code(error: &OrbitError) -> &'static str {
    match error {
        OrbitError::InvalidInput(_) | OrbitError::InvalidInputDiagnostic { .. } => "invalid_input",
        OrbitError::PolicyDenied(_) => "policy_denied",
        OrbitError::NotFound { .. } => "not_found",
        OrbitError::Store(_) => "store_error",
        OrbitError::Io(_) => "io_error",
        OrbitError::Migration(_) => "migration_failed",
        OrbitError::HubNegotiation(_) => "hub_negotiation",
        OrbitError::HubUnavailable(_) => "hub_unavailable",
        _ => "registration_failed",
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    #[test]
    fn private_request_rejects_identity_profile_mismatch_and_relative_roots() {
        let request = SpokeRegistrationRequestV1 {
            schema_version: SPOKE_REGISTRATION_SCHEMA_VERSION,
            identity: HostRegistration {
                machine_id: "hm_spoke".to_string(),
                host_id: "spoke".to_string(),
                labels: BTreeSet::new(),
            },
            presence: vec![WorkspacePresenceDeclaration {
                workspace_id: "ws_orbit".to_string(),
                root: "relative".into(),
                last_verified: Utc::now(),
            }],
            profiles: Vec::new(),
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn partial_result_cannot_smuggle_a_snapshot() {
        let result = SpokeRegistrationResultV1 {
            schema_version: SPOKE_REGISTRATION_SCHEMA_VERSION,
            complete: false,
            last_committed_stage: Some(SpokeRegistrationStageV1::Registry),
            host: None,
            presence_workspace_ids: Vec::new(),
            profile_workspace_ids: Vec::new(),
            snapshot: Some(RegistrySnapshotV1 {
                schema_version: super::super::REGISTRY_SNAPSHOT_SCHEMA_VERSION,
                hub_machine_id: Some("hm_hub".to_string()),
                registry_revision: 1,
                hosts: Vec::new(),
                workspaces: Vec::new(),
            }),
            failure: Some(SpokeRegistrationFailureV1 {
                code: "injected".to_string(),
                message: "injected".to_string(),
            }),
        };
        assert!(result.validate().is_err());
    }
}
