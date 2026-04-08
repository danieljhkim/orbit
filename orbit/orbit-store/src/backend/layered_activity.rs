use std::sync::Arc;

use orbit_types::{Activity, OrbitError};

use super::contracts::{ActivityCreateParams, ActivityStoreBackend, ActivityUpdateParams};

/// A layered activity store that merges a workspace store with a global store.
///
/// Read semantics: workspace entries shadow global entries by ID.
/// Write semantics: writes go to workspace store if present, otherwise global.
/// Mutations target whichever store owns the entry.
pub struct LayeredActivityStore {
    workspace: Arc<dyn ActivityStoreBackend>,
    global: Arc<dyn ActivityStoreBackend>,
}

impl LayeredActivityStore {
    pub fn new(
        workspace: Arc<dyn ActivityStoreBackend>,
        global: Arc<dyn ActivityStoreBackend>,
    ) -> Self {
        Self { workspace, global }
    }
}

impl ActivityStoreBackend for LayeredActivityStore {
    fn add_activity(&self, params: ActivityCreateParams) -> Result<Activity, OrbitError> {
        self.workspace.add_activity(params)
    }

    fn list_activities(&self, include_inactive: bool) -> Result<Vec<Activity>, OrbitError> {
        let workspace_activities = self.workspace.list_activities(include_inactive)?;
        let global_activities = self.global.list_activities(include_inactive)?;

        let workspace_ids: std::collections::HashSet<String> =
            workspace_activities.iter().map(|a| a.id.clone()).collect();

        let mut merged = workspace_activities;
        for activity in global_activities {
            if !workspace_ids.contains(&activity.id) {
                merged.push(activity);
            }
        }
        merged.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(merged)
    }

    fn get_activity(&self, id: &str) -> Result<Option<Activity>, OrbitError> {
        if let Some(activity) = self.workspace.get_activity(id)? {
            return Ok(Some(activity));
        }
        self.global.get_activity(id)
    }

    fn update_activity(
        &self,
        id: &str,
        params: ActivityUpdateParams,
    ) -> Result<Activity, OrbitError> {
        if self.workspace.get_activity(id)?.is_some() {
            return self.workspace.update_activity(id, params);
        }
        self.global.update_activity(id, params)
    }

    fn disable_activity(&self, id: &str) -> Result<bool, OrbitError> {
        if self.workspace.get_activity(id)?.is_some() {
            return self.workspace.disable_activity(id);
        }
        self.global.disable_activity(id)
    }
}
