use std::sync::Arc;

use orbit_types::{ExecutorDef, OrbitError};

use super::contracts::ExecutorDefStoreBackend;

/// A layered executor store that merges workspace definitions over global ones.
pub struct LayeredExecutorDefStore {
    workspace: Arc<dyn ExecutorDefStoreBackend>,
    global: Arc<dyn ExecutorDefStoreBackend>,
}

impl LayeredExecutorDefStore {
    pub fn new(
        workspace: Arc<dyn ExecutorDefStoreBackend>,
        global: Arc<dyn ExecutorDefStoreBackend>,
    ) -> Self {
        Self { workspace, global }
    }
}

impl ExecutorDefStoreBackend for LayeredExecutorDefStore {
    fn list_executor_defs(&self) -> Result<Vec<ExecutorDef>, OrbitError> {
        let workspace_defs = self.workspace.list_executor_defs()?;
        let global_defs = self.global.list_executor_defs()?;

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

    fn get_executor_def(&self, name: &str) -> Result<Option<ExecutorDef>, OrbitError> {
        if let Some(def) = self.workspace.get_executor_def(name)? {
            return Ok(Some(def));
        }
        self.global.get_executor_def(name)
    }

    fn upsert_executor_def(&self, def: &ExecutorDef) -> Result<(), OrbitError> {
        self.workspace.upsert_executor_def(def)
    }
}
