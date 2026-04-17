use std::sync::Arc;

use orbit_types::{OrbitError, PolicyDef};

use super::contracts::PolicyDefStoreBackend;

/// A layered policy store that merges workspace definitions over global ones.
pub struct LayeredPolicyDefStore {
    workspace: Arc<dyn PolicyDefStoreBackend>,
    global: Arc<dyn PolicyDefStoreBackend>,
}

impl LayeredPolicyDefStore {
    pub fn new(
        workspace: Arc<dyn PolicyDefStoreBackend>,
        global: Arc<dyn PolicyDefStoreBackend>,
    ) -> Self {
        Self { workspace, global }
    }
}

impl PolicyDefStoreBackend for LayeredPolicyDefStore {
    fn list_policy_defs(&self) -> Result<Vec<PolicyDef>, OrbitError> {
        let workspace_defs = self.workspace.list_policy_defs()?;
        let global_defs = self.global.list_policy_defs()?;

        let workspace_names: std::collections::HashSet<String> =
            workspace_defs.iter().map(|def| def.name.clone()).collect();

        let mut merged = workspace_defs;
        for def in global_defs {
            if !workspace_names.contains(&def.name) {
                merged.push(def);
            }
        }
        merged.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(merged)
    }

    fn get_policy_def(&self, name: &str) -> Result<Option<PolicyDef>, OrbitError> {
        if let Some(def) = self.workspace.get_policy_def(name)? {
            return Ok(Some(def));
        }
        self.global.get_policy_def(name)
    }

    fn upsert_policy_def(&self, def: &PolicyDef) -> Result<(), OrbitError> {
        self.workspace.upsert_policy_def(def)
    }
}
