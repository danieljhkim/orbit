use std::sync::Arc;

use orbit_common::OrbitError;
use orbit_types::policy::PolicyDef;

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
        let global_defs = self.global.list_policy_defs()?;
        let mut merged = self.workspace.list_policy_defs()?;
        for def in global_defs {
            if let Some(existing) = merged
                .iter_mut()
                .find(|candidate| candidate.name == def.name)
            {
                *existing = PolicyDef::merged(&def, existing)?;
            } else {
                merged.push(def);
            }
        }
        merged.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(merged)
    }

    fn get_policy_def(&self, name: &str) -> Result<Option<PolicyDef>, OrbitError> {
        let workspace = self.workspace.get_policy_def(name)?;
        let global = self.global.get_policy_def(name)?;
        match (global, workspace) {
            (Some(global), Some(workspace)) => Ok(Some(PolicyDef::merged(&global, &workspace)?)),
            (None, Some(workspace)) => Ok(Some(workspace)),
            (Some(global), None) => Ok(Some(global)),
            (None, None) => Ok(None),
        }
    }

    fn upsert_policy_def(&self, def: &PolicyDef) -> Result<(), OrbitError> {
        self.workspace.upsert_policy_def(def)
    }
}
