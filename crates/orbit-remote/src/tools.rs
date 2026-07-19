//! Remote-owned coordination discovery tools injected into Core runtimes.

use std::path::PathBuf;

use orbit_common::types::{NotFoundKind, OrbitError, ToolSessionContext};
use orbit_core::runtime::CoordinationToolDispatcher;
use serde_json::{Value, json};

use crate::service::registry_snapshot_at;

#[derive(Clone)]
pub(crate) struct RemoteCoordinationTools {
    global_root: PathBuf,
}

impl RemoteCoordinationTools {
    pub(crate) fn new(global_root: PathBuf) -> Self {
        Self { global_root }
    }
}

impl CoordinationToolDispatcher for RemoteCoordinationTools {
    fn execute_coordination_tool(
        &self,
        name: &str,
        _input: Value,
        _session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        let snapshot = registry_snapshot_at(&self.global_root)?;
        match name {
            "orbit.host.list" => Ok(json!({
                "hub_machine_id": snapshot.hub_machine_id,
                "registry_revision": snapshot.registry_revision,
                "hosts": snapshot.hosts,
            })),
            "orbit.workspace.list" => Ok(json!({
                "hub_machine_id": snapshot.hub_machine_id,
                "registry_revision": snapshot.registry_revision,
                "workspaces": snapshot.workspaces,
            })),
            _ => Err(OrbitError::not_found(NotFoundKind::Tool, name.to_string())),
        }
    }
}
