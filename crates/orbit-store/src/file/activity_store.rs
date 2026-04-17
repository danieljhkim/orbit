use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_types::{
    Activity, ActivityResource, ActivityResourceSpec, OrbitError, RESOURCE_SCHEMA_VERSION,
    ResourceKind,
};

use crate::backend::{ActivityCreateParams, ActivityUpdateParams};
use crate::file::layout::{DualLayout, file_timestamps};
use crate::file::sort::sort_by_created_desc_id_asc;
use crate::file::yaml_doc::{enumerate_yaml, read_yaml, write_yaml_atomic};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub(crate) struct ActivityFileStore {
    root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyActivitySpecDocument {
    id: String,
    spec_type: String,
    description: String,
    input_schema_json: Value,
    output_schema_json: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    executor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspace_path: Option<String>,
    #[serde(flatten)]
    spec_config: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyActivityFileDocument {
    schema_version: u8,
    #[serde(default)]
    created_by: Option<String>,
    #[allow(dead_code)]
    #[serde(default, rename = "identity_id", skip_serializing)]
    legacy_identity_id: Option<String>,
    activity: LegacyActivitySpecDocument,
}

impl ActivityFileStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn ensure_layout(&self) -> Result<(), OrbitError> {
        self.doc_layout().ensure()
    }

    pub(crate) fn insert_work(
        &self,
        params: &ActivityCreateParams,
    ) -> Result<Activity, OrbitError> {
        self.ensure_layout()?;
        if self.get_activity(&params.id)?.is_some() {
            return Err(OrbitError::InvalidInput(format!(
                "activity already exists: {}",
                params.id
            )));
        }

        let doc = LegacyActivityFileDocument {
            schema_version: 1,
            created_by: params.created_by.clone(),
            legacy_identity_id: None,
            activity: LegacyActivitySpecDocument {
                id: params.id.clone(),
                spec_type: params.spec_type.clone(),
                description: params.description.clone(),
                input_schema_json: normalize_json_schema_for_storage(
                    params.input_schema_json.clone(),
                ),
                output_schema_json: normalize_json_schema_for_storage(
                    params.output_schema_json.clone(),
                ),
                executor: params.executor.clone(),
                workspace_path: params.workspace_path.clone(),
                spec_config: params.spec_config.as_object().cloned().unwrap_or_default(),
            },
        };
        let path = self.active_doc_path(&doc.activity.id);
        self.write_doc_at(&path, &doc)?;
        self.read_activity_at(&path, true)
    }

    pub(crate) fn list_activities(
        &self,
        include_inactive: bool,
    ) -> Result<Vec<Activity>, OrbitError> {
        let layout = self.doc_layout();
        let mut activities = self.list_dir_docs(&layout.primary, true)?;
        if include_inactive {
            activities.extend(self.list_dir_docs(&layout.secondary, false)?);
        }
        sort_by_created_desc_id_asc(
            &mut activities,
            |activity| &activity.created_at,
            |activity| &activity.id,
        );
        Ok(activities)
    }

    pub(crate) fn get_activity(&self, id: &str) -> Result<Option<Activity>, OrbitError> {
        if let Some((path, is_active)) = self.doc_layout().locate(id, "yaml") {
            return Ok(Some(self.read_activity_at(&path, is_active)?));
        }
        Ok(None)
    }

    pub(crate) fn update_activity(
        &self,
        id: &str,
        params: &ActivityUpdateParams,
    ) -> Result<Activity, OrbitError> {
        self.ensure_layout()?;
        let layout = self.doc_layout();
        let Some((path, current_active)) = layout.locate(id, "yaml") else {
            return Err(OrbitError::InvalidInput(format!(
                "activity not found: {id}"
            )));
        };
        let mut doc = self.read_doc_at(&path)?;
        if let Some(v) = params.description.clone() {
            doc.activity.description = v;
        }
        if let Some(v) = params.input_schema_json.clone() {
            doc.activity.input_schema_json = normalize_json_schema_for_storage(v);
        }
        if let Some(v) = params.output_schema_json.clone() {
            doc.activity.output_schema_json = normalize_json_schema_for_storage(v);
        }
        if let Some(v) = params.spec_config.clone() {
            doc.activity.spec_config = v.as_object().cloned().unwrap_or_default();
        }
        if let Some(v) = params.executor.clone() {
            doc.activity.executor = v;
        }
        if let Some(v) = params.workspace_path.clone() {
            doc.activity.workspace_path = v;
        }
        if let Some(v) = params.created_by.clone() {
            doc.created_by = v;
        }
        let new_active = params.is_active.unwrap_or(current_active);
        if new_active != current_active {
            let new_path = if new_active {
                layout.primary_file(id, "yaml")
            } else {
                layout.secondary_file(id, "yaml")
            };
            self.write_doc_at(&new_path, &doc)?;
            fs::remove_file(&path).map_err(|e| OrbitError::Io(e.to_string()))?;
            return self.read_activity_at(&new_path, new_active);
        }
        self.write_doc_at(&path, &doc)?;
        self.read_activity_at(&path, new_active)
    }

    pub(crate) fn disable_activity(&self, id: &str) -> Result<bool, OrbitError> {
        self.ensure_layout()?;
        let layout = self.doc_layout();
        let active = layout.primary_file(id, "yaml");
        if active.exists() {
            let doc = self.read_doc_at(&active)?;
            let inactive = layout.secondary_file(id, "yaml");
            self.write_doc_at(&inactive, &doc)?;
            fs::remove_file(&active).map_err(|e| OrbitError::Io(e.to_string()))?;
            return Ok(true);
        }

        let inactive = layout.secondary_file(id, "yaml");
        if inactive.exists() {
            let doc = self.read_doc_at(&inactive)?;
            self.write_doc_at(&inactive, &doc)?;
            return Ok(true);
        }

        Ok(false)
    }

    fn list_dir_docs(&self, dir: &Path, active: bool) -> Result<Vec<Activity>, OrbitError> {
        enumerate_yaml(dir, "activity file", |path| {
            self.read_activity_at(&path, active)
        })
    }

    fn read_doc_at(&self, path: &Path) -> Result<LegacyActivityFileDocument, OrbitError> {
        let doc = read_yaml::<ActivityResource>(path, "activity file")?;
        legacy_doc_from_resource(doc, path)
    }

    fn read_activity_at(&self, path: &Path, is_active: bool) -> Result<Activity, OrbitError> {
        let doc = self.read_doc_at(path)?;
        let (created_at, updated_at) = file_timestamps(path)?;
        Ok(doc_to_work(doc, is_active, created_at, updated_at))
    }

    fn write_doc_at(
        &self,
        path: &Path,
        doc: &LegacyActivityFileDocument,
    ) -> Result<(), OrbitError> {
        let is_active = path.starts_with(self.doc_layout().primary.as_path());
        write_yaml_atomic(path, &resource_from_legacy_doc(doc, is_active))
    }

    fn doc_layout(&self) -> DualLayout {
        DualLayout {
            primary: self.root.join("active"),
            secondary: self.root.join("inactive"),
        }
    }

    fn active_doc_path(&self, id: &str) -> PathBuf {
        self.doc_layout().primary_file(id, "yaml")
    }
}

fn doc_to_work(
    doc: LegacyActivityFileDocument,
    is_active: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
) -> Activity {
    let tools: Vec<String> = doc
        .activity
        .spec_config
        .get("tools")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let proc_allowed_programs: Vec<String> = doc
        .activity
        .spec_config
        .get("proc_allowed_programs")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    Activity {
        id: doc.activity.id,
        spec_type: doc.activity.spec_type,
        description: doc.activity.description,
        input_schema_json: normalize_json_schema_for_runtime(doc.activity.input_schema_json),
        output_schema_json: normalize_json_schema_for_runtime(doc.activity.output_schema_json),
        spec_config: Value::Object(doc.activity.spec_config),
        tools,
        proc_allowed_programs,
        executor: doc.activity.executor,
        workspace_path: doc.activity.workspace_path,
        created_by: doc.created_by,
        is_active,
        created_at,
        updated_at,
    }
}

fn legacy_doc_from_resource(
    doc: ActivityResource,
    path: &Path,
) -> Result<LegacyActivityFileDocument, OrbitError> {
    if doc.kind != ResourceKind::Activity {
        return Err(OrbitError::Store(format!(
            "invalid activity file '{}': expected kind Activity, found {}",
            path.display(),
            doc.kind
        )));
    }
    if doc.schema_version != RESOURCE_SCHEMA_VERSION {
        return Err(OrbitError::Store(format!(
            "invalid activity file '{}': unsupported schemaVersion {}",
            path.display(),
            doc.schema_version
        )));
    }

    Ok(LegacyActivityFileDocument {
        schema_version: 1,
        created_by: doc.spec.created_by,
        legacy_identity_id: None,
        activity: LegacyActivitySpecDocument {
            id: doc.metadata.name,
            spec_type: doc.spec.spec_type,
            description: doc.spec.description,
            input_schema_json: normalize_json_schema_for_storage(doc.spec.input_schema_json),
            output_schema_json: normalize_json_schema_for_storage(doc.spec.output_schema_json),
            executor: doc.spec.executor,
            workspace_path: doc.spec.workspace_path,
            spec_config: doc.spec.spec_config,
        },
    })
}

fn resource_from_legacy_doc(doc: &LegacyActivityFileDocument, is_active: bool) -> ActivityResource {
    ActivityResource::new(
        ResourceKind::Activity,
        doc.activity.id.clone(),
        ActivityResourceSpec {
            spec_type: doc.activity.spec_type.clone(),
            description: doc.activity.description.clone(),
            input_schema_json: doc.activity.input_schema_json.clone(),
            output_schema_json: doc.activity.output_schema_json.clone(),
            executor: doc.activity.executor.clone(),
            workspace_path: doc.activity.workspace_path.clone(),
            created_by: doc.created_by.clone(),
            is_active,
            spec_config: doc.activity.spec_config.clone(),
        },
    )
}

fn normalize_json_schema_for_storage(value: serde_json::Value) -> serde_json::Value {
    rename_json_schema_key(value, "additionalProperties", "additional_properties")
}

fn normalize_json_schema_for_runtime(value: serde_json::Value) -> serde_json::Value {
    rename_json_schema_key(value, "additional_properties", "additionalProperties")
}

fn rename_json_schema_key(value: serde_json::Value, from: &str, to: &str) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let renamed = map
                .into_iter()
                .map(|(key, child)| {
                    let key = if key == from { to.to_string() } else { key };
                    (key, rename_json_schema_key(child, from, to))
                })
                .collect();
            serde_json::Value::Object(renamed)
        }
        serde_json::Value::Array(values) => serde_json::Value::Array(
            values
                .into_iter()
                .map(|child| rename_json_schema_key(child, from, to))
                .collect(),
        ),
        other => other,
    }
}
