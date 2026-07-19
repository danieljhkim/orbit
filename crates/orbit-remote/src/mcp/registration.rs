//! Remote-owned spoke bootstrap composition.

use std::collections::{BTreeMap, BTreeSet};

use crate::runtime::{RemoteRuntimeFactory, resolved_workspace_binding};
use crate::{HostIdentity, RegistryCacheService, RegistryCacheState, build_execution_profile_v1};
use chrono::{Duration, Utc};
use orbit_common::types::{
    HostRecord, HostRegistration, McpTransport, OrbitError, RegistrySnapshotV1,
    SPOKE_REGISTRATION_SCHEMA_VERSION, SpokeExecutionProfilePublicationV1,
    SpokeRegistrationRequestV1, ToolSessionContext, WorkspaceCheckoutRole,
    WorkspacePresenceDeclaration, WorkspaceStatus, audit_execution_id,
};
use orbit_core::OrbitRuntime;

use super::config::load_trusted_mcp_config;
use super::host::canonical_mcp_tool_definitions;
use super::hub_link::HubLinkPool;

/// Register one validated local spoke identity through the already-pinned hub
/// route and refresh the sanitized local cache only after complete hub success.
pub fn register_local_spoke(
    runtime: &OrbitRuntime,
    identity: &HostIdentity,
    labels: BTreeSet<String>,
) -> Result<HostRecord, OrbitError> {
    let global_root = runtime.global_root();
    let trusted = load_trusted_mcp_config(&global_root)?;
    let (route, capability) = trusted.spoke_registration_route(identity)?;
    let registry_path = crate::workspace_registry::registry_path_for(&global_root);
    let registry = crate::workspace_registry::load_registry_from(&registry_path)?;
    let observed_at = Utc::now();

    let mut presence = crate::workspace_registry::local_workspaces(&registry)
        .filter(|(workspace, checkout)| {
            workspace.status == WorkspaceStatus::Active && checkout.repo_root.exists()
        })
        .map(|(workspace, checkout)| WorkspacePresenceDeclaration {
            workspace_id: workspace.id.clone(),
            root: checkout.repo_root.clone(),
            last_verified: observed_at,
        })
        .collect::<Vec<_>>();
    presence.sort_by(|left, right| left.workspace_id.cmp(&right.workspace_id));

    let cached_snapshot = cached_snapshot(&global_root)?;
    let mut profiles = Vec::new();
    for (workspace, checkout) in crate::workspace_registry::local_workspaces(&registry) {
        if workspace.status != WorkspaceStatus::Active
            || checkout.role != Some(WorkspaceCheckoutRole::Owner)
            || workspace.owner_machine_id.as_deref() != Some(identity.machine_id.as_str())
        {
            continue;
        }
        let binding = resolved_workspace_binding(workspace, checkout)?;
        let owned_runtime = if runtime.shared_root() == checkout.orbit_dir {
            runtime.clone()
        } else {
            RemoteRuntimeFactory::open_registered_checkout(&global_root, workspace, checkout)?
        };
        let profile = build_execution_profile_v1(
            owned_runtime.execution_environment_snapshot()?,
            workspace,
            &binding,
            &identity.machine_id,
            observed_at,
        )?;
        let expected_generation = cached_snapshot
            .as_ref()
            .and_then(|snapshot| {
                snapshot
                    .workspaces
                    .iter()
                    .find(|entry| entry.workspace_id == workspace.id)
            })
            .and_then(|entry| entry.profile.generation)
            .unwrap_or(0);
        profiles.push(SpokeExecutionProfilePublicationV1 {
            expected_generation,
            profile,
        });
    }
    profiles.sort_by(|left, right| left.profile.workspace_id.cmp(&right.profile.workspace_id));

    let request = SpokeRegistrationRequestV1 {
        schema_version: SPOKE_REGISTRATION_SCHEMA_VERSION,
        identity: HostRegistration {
            machine_id: identity.machine_id.clone(),
            host_id: identity.host_id.clone(),
            labels,
        },
        presence,
        profiles,
    };
    request.validate()?;

    let definitions = canonical_mcp_tool_definitions()
        .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;
    let schema_digest = orbit_mcp::hub_schema_digest(&definitions, capability)?;
    let pool = HubLinkPool::ssh(
        route.host.clone(),
        route.machine_id.clone(),
        BTreeMap::from([(capability, schema_digest)]),
    )?;
    let context = ToolSessionContext {
        caller_machine_id: Some(identity.machine_id.clone()),
        caller_host_id: Some(identity.host_id.clone()),
        transport: Some(McpTransport::SshMcp),
        effective_capabilities: BTreeSet::from([capability]),
        origin_session_id: Some(audit_execution_id("host-register-session")),
        mcp_call_id: Some(audit_execution_id("mcall-register-spoke")),
        ..ToolSessionContext::default()
    };
    let result = pool.register_spoke(capability, request, context)?;
    if !result.complete {
        let failure = result.failure.as_ref().ok_or_else(|| {
            OrbitError::HubNegotiation(
                "hub returned an incomplete registration result without a failure".to_string(),
            )
        })?;
        let stage = result
            .last_committed_stage
            .map(|stage| format!("{stage:?}").to_lowercase())
            .unwrap_or_else(|| "none".to_string());
        return Err(OrbitError::RemoteTool {
            code: failure.code.clone(),
            message: format!(
                "spoke registration stopped after committed stage '{stage}': {}",
                failure.message
            ),
            payload: serde_json::to_value(&result).unwrap_or_default(),
        });
    }
    let snapshot = result.snapshot.ok_or_else(|| {
        OrbitError::HubNegotiation(
            "hub returned complete registration without a sanitized snapshot".to_string(),
        )
    })?;
    let host = result.host.ok_or_else(|| {
        OrbitError::HubNegotiation(
            "hub returned complete registration without the registered host".to_string(),
        )
    })?;
    RegistryCacheService::new(&global_root)
        .refresh(snapshot, Utc::now())
        .map_err(|error| {
            OrbitError::Store(format!(
                "hub registration completed through sanitized snapshot, but local registry cache refresh failed without rolling back hub state: {error}"
            ))
        })?;
    Ok(host)
}

fn cached_snapshot(
    global_root: &std::path::Path,
) -> Result<Option<RegistrySnapshotV1>, OrbitError> {
    match RegistryCacheService::new(global_root).load(Utc::now(), Duration::zero())? {
        RegistryCacheState::Current { cache, .. } | RegistryCacheState::Stale { cache, .. } => {
            Ok(Some(cache.snapshot))
        }
        RegistryCacheState::Missing
        | RegistryCacheState::Malformed { .. }
        | RegistryCacheState::UnsupportedFuture { .. } => Ok(None),
    }
}
