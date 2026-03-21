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

        let workspace_ids: std::collections::HashSet<String> = workspace_activities
            .iter()
            .map(|a| a.id.clone())
            .collect();

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::memory_activity::MemoryActivityStoreBackend;

    fn make_params(id: &str) -> ActivityCreateParams {
        ActivityCreateParams {
            id: id.to_string(),
            spec_type: "shell".to_string(),
            description: format!("desc-{id}"),
            input_schema_json: serde_json::Value::Null,
            output_schema_json: serde_json::Value::Null,
            spec_config: serde_json::Value::Null,
            workspace_path: None,
            created_by: None,
        }
    }

    fn make_layered() -> (
        Arc<MemoryActivityStoreBackend>,
        Arc<MemoryActivityStoreBackend>,
        LayeredActivityStore,
    ) {
        let ws = Arc::new(MemoryActivityStoreBackend::default());
        let global = Arc::new(MemoryActivityStoreBackend::default());
        let layered = LayeredActivityStore::new(ws.clone(), global.clone());
        (ws, global, layered)
    }

    #[test]
    fn workspace_shadows_global_by_id() {
        let (ws, global, layered) = make_layered();
        global.add_activity(make_params("shared")).unwrap();
        ws.add_activity(make_params("shared")).unwrap();

        let activities = layered.list_activities(true).unwrap();
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].id, "shared");
    }

    #[test]
    fn global_only_entries_visible() {
        let (_ws, global, layered) = make_layered();
        global.add_activity(make_params("global-only")).unwrap();

        let result = layered.get_activity("global-only").unwrap();
        assert!(result.is_some());

        let activities = layered.list_activities(true).unwrap();
        assert_eq!(activities.len(), 1);
    }

    #[test]
    fn workspace_only_entries_visible() {
        let (ws, _global, layered) = make_layered();
        ws.add_activity(make_params("ws-only")).unwrap();

        let result = layered.get_activity("ws-only").unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn merge_returns_union() {
        let (ws, global, layered) = make_layered();
        global.add_activity(make_params("g1")).unwrap();
        global.add_activity(make_params("g2")).unwrap();
        ws.add_activity(make_params("w1")).unwrap();
        ws.add_activity(make_params("g2")).unwrap(); // shadows global g2

        let activities = layered.list_activities(true).unwrap();
        assert_eq!(activities.len(), 3); // g1, g2 (from ws), w1
    }

    #[test]
    fn writes_go_to_workspace() {
        let (ws, global, layered) = make_layered();
        layered.add_activity(make_params("new")).unwrap();

        assert!(ws.get_activity("new").unwrap().is_some());
        assert!(global.get_activity("new").unwrap().is_none());
    }

    #[test]
    fn update_targets_owning_store() {
        let (ws, global, layered) = make_layered();
        global.add_activity(make_params("gact")).unwrap();
        ws.add_activity(make_params("wact")).unwrap();

        let update = ActivityUpdateParams {
            description: Some("updated".to_string()),
            ..Default::default()
        };
        layered.update_activity("gact", update.clone()).unwrap();
        layered.update_activity("wact", update).unwrap();

        assert_eq!(
            global.get_activity("gact").unwrap().unwrap().description,
            "updated"
        );
        assert_eq!(
            ws.get_activity("wact").unwrap().unwrap().description,
            "updated"
        );
    }

    #[test]
    fn disable_targets_owning_store() {
        let (_ws, global, layered) = make_layered();
        global.add_activity(make_params("gact")).unwrap();

        assert!(layered.disable_activity("gact").unwrap());
    }
}
