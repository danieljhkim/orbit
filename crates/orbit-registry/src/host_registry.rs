//! Typed domain service for the hub host registry [ORB-10255].
//!
//! Dormant v2 substrate, currently unreachable from v1 execution paths. It is
//! retained for its tombstone-alias and retirement semantics.
//!
//! This layer binds B1's stable local [`HostIdentity`] declaration to the
//! durable hub-store API. It intentionally does not coordinate local
//! `host.toml` renames, expose administration commands, or add transport;
//! those surfaces belong to the later registry-administration unit.

use std::collections::BTreeSet;
use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use orbit_common::types::{
    HostAlias, HostNameResolution, HostRecord, HostRegistration, HostWorkspacePresence, OrbitError,
    RegistrySnapshotV1, SanitizedWorkspacePresence, Workspace, WorkspaceOwnership,
    WorkspacePresenceDeclaration, WorkspaceRegistry, WorkspaceStatus,
};

use crate::host_identity::{HostIdentity, HostMode, load_host_identity};
use crate::persistence::RegistryStore;

/// Freshness horizon the dormant registry snapshot still reports profile rows
/// with. No v1 caller publishes a profile; the snapshot keeps a
/// single TTL so the dormant projection reads consistently.
const PROFILE_FRESHNESS_TTL: Duration = Duration::minutes(10);
const PRESENCE_FRESHNESS_TTL: Duration = Duration::minutes(5);

#[derive(Clone)]
pub struct HostRegistryService {
    store: RegistryStore,
}

/// Result of a hub-side workspace owner link: the recorded singular ownership
/// plus an optional visible warning when the owner name was resolved through a
/// permanent tombstone alias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceLink {
    pub ownership: WorkspaceOwnership,
    pub warning: Option<String>,
}

/// Load the machine-local identity and fail closed unless this process is
/// running on the declared coordination hub. CLI host administration and
/// workspace ownership binding are hub-local in v1; a spoke must reach these
/// operations through its configured hub route rather than opening its local
/// coordination store.
pub fn require_local_hub_identity(global_root: &Path) -> Result<HostIdentity, OrbitError> {
    let identity = load_host_identity(global_root)?;
    if identity.mode != HostMode::Hub {
        return Err(OrbitError::InvalidInput(format!(
            "host-registry administration is hub-local in v1; this machine '{}' ({}) is configured as mode '{}'. Run the command on the coordination hub",
            identity.host_id, identity.machine_id, identity.mode
        )));
    }
    Ok(identity)
}

impl HostRegistryService {
    pub fn new(store: RegistryStore) -> Self {
        Self { store }
    }

    /// Register B1's stable machine identity with a compatible label set.
    pub fn register_identity(
        &self,
        identity: &HostIdentity,
        labels: BTreeSet<String>,
    ) -> Result<HostRecord, OrbitError> {
        self.store.register_host(&HostRegistration {
            machine_id: identity.machine_id.clone(),
            host_id: identity.host_id.clone(),
            labels,
        })
    }

    /// Atomically register this machine as the singular coordination hub and
    /// stamp the hub identity into the sanitized registry snapshot.
    pub fn register_hub_identity(
        &self,
        identity: &HostIdentity,
        labels: BTreeSet<String>,
    ) -> Result<HostRecord, OrbitError> {
        if identity.mode != HostMode::Hub {
            return Err(OrbitError::InvalidInput(format!(
                "cannot register machine_id '{}' as the hub while host.toml mode is '{}'",
                identity.machine_id, identity.mode
            )));
        }
        self.store.register_hub(&HostRegistration {
            machine_id: identity.machine_id.clone(),
            host_id: identity.host_id.clone(),
            labels,
        })
    }

    /// Verify that this opened coordination store belongs to the same hub as
    /// the machine-local `host.toml`. This rejects hub-mode shadow databases
    /// and unbootstrapped stores before an administration command can mutate
    /// them. Only [`Self::register_hub_identity`] may bootstrap a missing stamp.
    pub fn require_configured_local_hub(&self, identity: &HostIdentity) -> Result<(), OrbitError> {
        if identity.mode != HostMode::Hub {
            return Err(OrbitError::InvalidInput(format!(
                "host-registry administration requires hub mode, not '{}'",
                identity.mode
            )));
        }
        match self.store.hub_machine_id()? {
            Some(configured) if configured == identity.machine_id => Ok(()),
            Some(configured) => Err(OrbitError::InvalidInput(format!(
                "refusing host-registry administration through a shadow coordination store: local hub machine_id '{}' does not match configured hub machine_id '{configured}'",
                identity.machine_id
            ))),
            None => Err(OrbitError::InvalidInput(
                "the coordination store has no configured hub identity; run `orbit host register` without --machine-id/--host-id on this hub first"
                    .to_string(),
            )),
        }
    }

    pub fn rename(&self, machine_id: &str, new_host_id: &str) -> Result<HostRecord, OrbitError> {
        self.store.rename_host(machine_id, new_host_id)
    }

    /// Run the exact store-side rename validation without mutating the
    /// registry. Current-machine administration uses this before replacing
    /// `host.toml`; [`Self::rename`] repeats it transactionally afterward.
    pub fn validate_rename(
        &self,
        machine_id: &str,
        new_host_id: &str,
    ) -> Result<HostRecord, OrbitError> {
        self.store.validate_host_rename(machine_id, new_host_id)
    }

    /// Read one machine by immutable ID. This is also the post-error probe for
    /// classifying an uncertain registry rename commit.
    pub fn host(&self, machine_id: &str) -> Result<Option<HostRecord>, OrbitError> {
        self.store.get_host(machine_id)
    }

    pub fn retire(&self, machine_id: &str) -> Result<HostRecord, OrbitError> {
        self.store.retire_host(machine_id)
    }

    /// Retire a machine, rejecting an attempt to retire the singular configured
    /// hub machine atomically with the retirement mutation. In v1 there is
    /// exactly one hub and it cannot retire itself out of existence.
    pub fn retire_guarding_hub(&self, machine_id: &str) -> Result<HostRecord, OrbitError> {
        self.store.retire_host(machine_id)
    }

    pub fn resolve(&self, host_id: &str) -> Result<HostNameResolution, OrbitError> {
        self.store.resolve_host_id(host_id)
    }

    /// The configured hub `machine_id`, if any.
    pub fn hub_machine_id(&self) -> Result<Option<String>, OrbitError> {
        self.store.hub_machine_id()
    }

    /// Read the single sanitized, path-free registry snapshot — the sole input
    /// to the discovery tools and the satellite registry cache.
    pub fn snapshot(&self) -> Result<RegistrySnapshotV1, OrbitError> {
        self.store
            .read_registry_snapshot(Utc::now(), PRESENCE_FRESHNESS_TTL, PROFILE_FRESHNESS_TTL)
    }

    /// Resolve a human owner name through C1 and record C2's singular owner
    /// `machine_id`. Active names bind silently; a tombstone alias binds with a
    /// visible warning; unknown, retired, and collision results fail before any
    /// ownership mutation. No SSH target, capability grant, coordination host,
    /// or inferred owner is ever recorded.
    pub fn link_workspace_owner(
        &self,
        registry: &WorkspaceRegistry,
        workspace_id: &str,
        owner_host_id: &str,
    ) -> Result<WorkspaceLink, OrbitError> {
        let (owner_machine_id, warning) = match self.store.resolve_host_id(owner_host_id)? {
            HostNameResolution::Active { host } => (host.machine_id, None),
            HostNameResolution::Alias { host, alias } => (host.machine_id, Some(alias.warning)),
            HostNameResolution::Retired { host, .. } => {
                return Err(OrbitError::InvalidInput(format!(
                    "owner name '{owner_host_id}' resolves to retired machine_id '{}'; a retired \
                     host cannot own a workspace",
                    host.machine_id
                )));
            }
            HostNameResolution::Unknown { host_id } => {
                return Err(OrbitError::InvalidInput(format!(
                    "owner name '{host_id}' is not a registered host"
                )));
            }
            HostNameResolution::Collision {
                host_id,
                machine_ids,
            } => {
                return Err(OrbitError::InvalidInput(format!(
                    "owner name '{host_id}' is ambiguous across machine_ids [{}]; refusing to bind",
                    machine_ids.join(", ")
                )));
            }
        };
        let ownership = self.bind_workspace_owner(registry, workspace_id, &owner_machine_id)?;
        Ok(WorkspaceLink { ownership, warning })
    }

    pub fn active_hosts(&self) -> Result<Vec<HostRecord>, OrbitError> {
        self.store.list_active_hosts()
    }

    pub fn aliases(&self, machine_id: &str) -> Result<Vec<HostAlias>, OrbitError> {
        self.store.list_host_aliases(machine_id)
    }

    pub fn bind_workspace_owner(
        &self,
        registry: &WorkspaceRegistry,
        workspace_id: &str,
        owner_machine_id: &str,
    ) -> Result<WorkspaceOwnership, OrbitError> {
        let workspace = require_logical_workspace(registry, workspace_id)?;
        if let Some(mirror) = workspace.owner_machine_id.as_deref()
            && mirror != owner_machine_id
        {
            return Err(OrbitError::InvalidInput(format!(
                "workspace_id '{workspace_id}' local owner mirror '{mirror}' does not match requested hub owner '{owner_machine_id}'"
            )));
        }
        self.store
            .bind_workspace_owner(workspace_id, owner_machine_id)
    }

    pub fn publish_presence(
        &self,
        registry: &WorkspaceRegistry,
        caller_machine_id: &str,
        declarations: &[WorkspacePresenceDeclaration],
    ) -> Result<Vec<HostWorkspacePresence>, OrbitError> {
        self.publish_presence_at(registry, caller_machine_id, declarations, Utc::now())
    }

    fn publish_presence_at(
        &self,
        registry: &WorkspaceRegistry,
        caller_machine_id: &str,
        declarations: &[WorkspacePresenceDeclaration],
        received_at: DateTime<Utc>,
    ) -> Result<Vec<HostWorkspacePresence>, OrbitError> {
        for declaration in declarations {
            require_logical_workspace(registry, &declaration.workspace_id)?;
        }
        self.store
            .replace_host_workspace_presence(caller_machine_id, declarations, received_at)
    }

    pub fn presence_status(
        &self,
        machine_id: &str,
        workspace_id: &str,
    ) -> Result<SanitizedWorkspacePresence, OrbitError> {
        self.store.sanitized_workspace_presence(
            machine_id,
            workspace_id,
            Utc::now(),
            PRESENCE_FRESHNESS_TTL,
        )
    }

    // The owner execution-profile publication surface is dormant. MCP crew
    // discovery reads the selected workspace's local config on the accepting
    // machine, so this service neither publishes nor reads that projection.
}

fn require_logical_workspace<'a>(
    registry: &'a WorkspaceRegistry,
    workspace_id: &str,
) -> Result<&'a Workspace, OrbitError> {
    let workspace = registry
        .workspaces
        .iter()
        .find(|workspace| workspace.id == workspace_id)
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!("unknown logical workspace_id '{workspace_id}'"))
        })?;
    if workspace.status != WorkspaceStatus::Active {
        return Err(OrbitError::InvalidInput(format!(
            "logical workspace_id '{workspace_id}' is not active"
        )));
    }
    Ok(workspace)
}
