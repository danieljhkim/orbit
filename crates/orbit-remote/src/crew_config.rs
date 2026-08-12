//! Owner-local crew discovery and explicit task-crew validation [ORB-10729].
//!
//! This is the single reusable service that both `orbit.crew.list` discovery
//! and explicit task-crew validation read through. It reads the owner machine's
//! own layered crew configuration directly (mcp-bridge §8.1) — never a
//! published projection, a satellite registry cache, a stale replica, or a
//! synchronous call to another machine.
//!
//! [ORB-10276] previously answered both questions from an owner-published
//! `ExecutionProfileV1` projection, with a generation counter and a freshness
//! TTL. Publication rode the registration/poll protocol, which is withdrawn
//! ([ADR-0358]), and in v1 crew validation runs *where the workspace is owned*.
//! Reading the local file needs no projection, no generation, and no freshness
//! gate, so all three are gone: a config the caller can read is current by
//! construction. `config_digest`, `ship_closure_digest`, and
//! [`crate::build_execution_profile_v1`] are transport-independent and survive
//! that withdrawal intact.

use std::path::PathBuf;

use orbit_common::types::{
    CREW_DISCOVERY_SCHEMA_VERSION, CrewDiscoveryV1, ExecutionProfileCrewV1, OrbitError,
    resolve_crew,
};
use orbit_core::local_crew_environment;

use crate::{HostIdentityState, inspect_host_identity};

/// Crew configuration as read on the machine that owns the workspace.
///
/// Every entry point is workspace-scoped because the config is: a workspace
/// with a local checkout layers `<checkout>/.orbit/config.toml` over
/// `<global_root>/config.toml`, exactly as its runtime would.
pub struct OwnerLocalCrews {
    global_root: PathBuf,
}

impl OwnerLocalCrews {
    pub fn new(global_root: PathBuf) -> Self {
        Self { global_root }
    }

    /// The sanitized `orbit.crew.list` projection for one workspace.
    pub fn crew_discovery(&self, workspace_id: &str) -> Result<CrewDiscoveryV1, OrbitError> {
        let environment =
            local_crew_environment(&self.global_root, &self.config_root(workspace_id))?;
        let crews = environment
            .crews
            .values()
            .map(|crew| ExecutionProfileCrewV1::from_crew(crew, environment.resolved_backend))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CrewDiscoveryV1 {
            schema_version: CREW_DISCOVERY_SCHEMA_VERSION,
            workspace_id: workspace_id.to_string(),
            owner_machine_id: self.owner_machine_id(workspace_id),
            default_crew: environment.default_crew,
            crews,
        })
    }

    /// Validate an explicit, non-empty task crew and return the exact alias the
    /// owner's configuration defines.
    ///
    /// An unknown crew names the workspace, the owning machine, and the crews
    /// that *are* configured, so the caller can either pick one of those or add
    /// the missing crew on the owner machine.
    pub fn validate_task_crew(&self, workspace_id: &str, crew: &str) -> Result<String, OrbitError> {
        let environment =
            local_crew_environment(&self.global_root, &self.config_root(workspace_id))?;
        resolve_crew(crew, &environment.crews)
            .map(|resolved| resolved.name)
            .map_err(|_| {
                OrbitError::InvalidInput(format!(
                    "crew '{crew}' is not configured for workspace '{workspace_id}' on its owner machine '{owner}'; configured crews: [{known}]. Add it to [crews] in that machine's config.toml, or file the task without a crew",
                    owner = self.owner_machine_id(workspace_id).unwrap_or_else(|| "unknown".to_string()),
                    known = environment
                        .crews
                        .keys()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })
    }

    /// The `.orbit` directory whose `config.toml` layers over the machine-global
    /// one, or the global root itself when the workspace has no local checkout
    /// (layering then reads the global file alone).
    fn config_root(&self, workspace_id: &str) -> PathBuf {
        self.registered_orbit_dir(workspace_id)
            .unwrap_or_else(|| self.global_root.clone())
    }

    fn registered_orbit_dir(&self, workspace_id: &str) -> Option<PathBuf> {
        let registry_path = crate::workspace_registry::registry_path_for(&self.global_root);
        let registry = crate::workspace_registry::load_registry_from(&registry_path).ok()?;
        registry
            .checkouts
            .iter()
            .find(|checkout| checkout.workspace_id == workspace_id)
            .map(|checkout| checkout.orbit_dir.clone())
            .filter(|orbit_dir| orbit_dir.is_dir())
    }

    /// The machine this workspace declares as its owner, falling back to this
    /// machine's identity for a workspace that predates the ownership model.
    fn owner_machine_id(&self, workspace_id: &str) -> Option<String> {
        let registry_path = crate::workspace_registry::registry_path_for(&self.global_root);
        crate::workspace_registry::load_registry_from(&registry_path)
            .ok()
            .and_then(|registry| {
                registry
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.id == workspace_id)
                    .and_then(|workspace| workspace.owner_machine_id.clone())
            })
            .or_else(|| self.local_machine_id())
    }

    fn local_machine_id(&self) -> Option<String> {
        match inspect_host_identity(&self.global_root) {
            Ok(HostIdentityState::Present(identity)) => Some(identity.machine_id),
            _ => None,
        }
    }
}
