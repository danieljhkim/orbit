use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use orbit_types::{Activity, OrbitError};
use serde_json::Value;

use super::contracts::{ActivityCreateParams, ActivityStoreBackend, ActivityUpdateParams};

#[derive(Clone, Default)]
pub struct MemoryActivityStoreBackend {
    inner: Arc<Mutex<HashMap<String, Activity>>>,
}

fn lock_err<T>(e: std::sync::PoisonError<T>) -> OrbitError {
    OrbitError::Store(format!("mutex poisoned: {e}"))
}

fn extract_tools(spec_config: &Value) -> Vec<String> {
    spec_config
        .get("tools")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

impl ActivityStoreBackend for MemoryActivityStoreBackend {
    fn add_activity(&self, params: ActivityCreateParams) -> Result<Activity, OrbitError> {
        let mut store = self.inner.lock().map_err(lock_err)?;
        if store.contains_key(&params.id) {
            return Err(OrbitError::InvalidInput(format!(
                "activity already exists: {}",
                params.id
            )));
        }
        let now = Utc::now();
        let tools = extract_tools(&params.spec_config);
        let activity = Activity {
            id: params.id.clone(),
            spec_type: params.spec_type,
            description: params.description,
            input_schema_json: params.input_schema_json,
            output_schema_json: params.output_schema_json,
            spec_config: params.spec_config,
            tools,
            workspace_path: params.workspace_path,
            created_by: params.created_by,
            is_active: true,
            created_at: now,
            updated_at: now,
        };
        store.insert(params.id, activity.clone());
        Ok(activity)
    }

    fn list_activities(&self, include_inactive: bool) -> Result<Vec<Activity>, OrbitError> {
        let store = self.inner.lock().map_err(lock_err)?;
        let mut activities: Vec<Activity> = store
            .values()
            .filter(|a| include_inactive || a.is_active)
            .cloned()
            .collect();
        activities.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(activities)
    }

    fn get_activity(&self, id: &str) -> Result<Option<Activity>, OrbitError> {
        let store = self.inner.lock().map_err(lock_err)?;
        Ok(store.get(id).cloned())
    }

    fn update_activity(
        &self,
        id: &str,
        params: ActivityUpdateParams,
    ) -> Result<Activity, OrbitError> {
        let mut store = self.inner.lock().map_err(lock_err)?;
        let Some(activity) = store.get_mut(id) else {
            return Err(OrbitError::InvalidInput(format!(
                "activity not found: {id}"
            )));
        };
        if let Some(v) = params.description {
            activity.description = v;
        }
        if let Some(v) = params.input_schema_json {
            activity.input_schema_json = v;
        }
        if let Some(v) = params.output_schema_json {
            activity.output_schema_json = v;
        }
        if let Some(v) = params.spec_config {
            activity.tools = extract_tools(&v);
            activity.spec_config = v;
        }
        if let Some(v) = params.workspace_path {
            activity.workspace_path = v;
        }
        if let Some(v) = params.created_by {
            activity.created_by = v;
        }
        if let Some(v) = params.is_active {
            activity.is_active = v;
        }
        activity.updated_at = Utc::now();
        Ok(activity.clone())
    }

    fn disable_activity(&self, id: &str) -> Result<bool, OrbitError> {
        let mut store = self.inner.lock().map_err(lock_err)?;
        let Some(activity) = store.get_mut(id) else {
            return Ok(false);
        };
        activity.is_active = false;
        activity.updated_at = Utc::now();
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::MemoryActivityStoreBackend;
    use crate::backend::contracts::{
        ActivityCreateParams, ActivityStoreBackend, ActivityUpdateParams,
    };

    fn sample_params(id: &str) -> ActivityCreateParams {
        ActivityCreateParams {
            id: id.to_string(),
            spec_type: "agent_invoke".to_string(),
            description: "Test activity".to_string(),
            input_schema_json: json!({"type": "object"}),
            output_schema_json: json!({"type": "object"}),
            spec_config: json!({"tools": ["fs.read"]}),
            workspace_path: None,
            created_by: Some("human".to_string()),
        }
    }

    #[test]
    fn add_and_get_activity_roundtrip() {
        let store = MemoryActivityStoreBackend::default();
        let activity = store.add_activity(sample_params("test_act")).expect("add");
        assert!(activity.is_active);
        assert_eq!(activity.tools, vec!["fs.read"]);

        let got = store
            .get_activity("test_act")
            .expect("get")
            .expect("exists");
        assert_eq!(got.id, "test_act");
        assert_eq!(got.created_by.as_deref(), Some("human"));
    }

    #[test]
    fn add_activity_rejects_duplicate_id() {
        let store = MemoryActivityStoreBackend::default();
        store.add_activity(sample_params("dup")).expect("first");
        let err = store.add_activity(sample_params("dup")).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn list_activities_respects_include_inactive() {
        let store = MemoryActivityStoreBackend::default();
        store
            .add_activity(sample_params("active"))
            .expect("add active");
        store
            .add_activity(sample_params("inactive"))
            .expect("add inactive");
        store.disable_activity("inactive").expect("disable");

        let active_only = store.list_activities(false).expect("list active");
        assert_eq!(active_only.len(), 1);
        assert_eq!(active_only[0].id, "active");

        let all = store.list_activities(true).expect("list all");
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn update_activity_changes_fields() {
        let store = MemoryActivityStoreBackend::default();
        store.add_activity(sample_params("upd")).expect("add");

        let updated = store
            .update_activity(
                "upd",
                ActivityUpdateParams {
                    description: Some("Updated desc".to_string()),
                    spec_config: Some(json!({"tools": ["fs.write"]})),
                    ..Default::default()
                },
            )
            .expect("update");

        assert_eq!(updated.description, "Updated desc");
        assert_eq!(updated.tools, vec!["fs.write"]);
    }

    #[test]
    fn disable_activity_returns_false_for_missing() {
        let store = MemoryActivityStoreBackend::default();
        assert!(!store.disable_activity("nonexistent").expect("disable"));
    }
}
