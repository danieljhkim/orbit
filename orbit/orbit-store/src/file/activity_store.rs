use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_types::{Activity, OrbitError};

use crate::backend::{ActivityCreateParams, ActivityUpdateParams};
use crate::file::fs_utils::write_atomic;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub(crate) struct ActivityFileStore {
    root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActivitySpecDocument {
    id: String,
    spec_type: String,
    description: String,
    input_schema_json: Value,
    output_schema_json: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspace_path: Option<String>,
    #[serde(flatten)]
    spec_config: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActivityFileDocument {
    schema_version: u8,
    #[serde(default)]
    created_by: Option<String>,
    #[allow(dead_code)]
    #[serde(default, rename = "identity_id", skip_serializing)]
    legacy_identity_id: Option<String>,
    activity: ActivitySpecDocument,
}

impl ActivityFileStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn ensure_layout(&self) -> Result<(), OrbitError> {
        fs::create_dir_all(self.active_dir()).map_err(|e| OrbitError::Io(e.to_string()))?;
        fs::create_dir_all(self.inactive_dir()).map_err(|e| OrbitError::Io(e.to_string()))?;
        Ok(())
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

        let doc = ActivityFileDocument {
            schema_version: 1,
            created_by: params.created_by.clone(),
            legacy_identity_id: None,
            activity: ActivitySpecDocument {
                id: params.id.clone(),
                spec_type: params.spec_type.clone(),
                description: params.description.clone(),
                input_schema_json: normalize_json_schema_for_storage(
                    params.input_schema_json.clone(),
                ),
                output_schema_json: normalize_json_schema_for_storage(
                    params.output_schema_json.clone(),
                ),
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
        let mut activities = self.list_dir_docs(&self.active_dir(), true)?;
        if include_inactive {
            activities.extend(self.list_dir_docs(&self.inactive_dir(), false)?);
        }
        activities.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(activities)
    }

    pub(crate) fn get_activity(&self, id: &str) -> Result<Option<Activity>, OrbitError> {
        let active = self.active_doc_path(id);
        if active.exists() {
            return Ok(Some(self.read_activity_at(&active, true)?));
        }
        let inactive = self.inactive_doc_path(id);
        if inactive.exists() {
            return Ok(Some(self.read_activity_at(&inactive, false)?));
        }
        Ok(None)
    }

    pub(crate) fn update_activity(
        &self,
        id: &str,
        params: &ActivityUpdateParams,
    ) -> Result<Activity, OrbitError> {
        self.ensure_layout()?;
        let (path, current_active) = if self.active_doc_path(id).exists() {
            (self.active_doc_path(id), true)
        } else if self.inactive_doc_path(id).exists() {
            (self.inactive_doc_path(id), false)
        } else {
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
        if let Some(v) = params.workspace_path.clone() {
            doc.activity.workspace_path = v;
        }
        if let Some(v) = params.created_by.clone() {
            doc.created_by = v;
        }
        let new_active = params.is_active.unwrap_or(current_active);
        if new_active != current_active {
            // Move the file to the new location.
            let new_path = if new_active {
                self.active_doc_path(id)
            } else {
                self.inactive_doc_path(id)
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
        let active = self.active_doc_path(id);
        if active.exists() {
            let doc = self.read_doc_at(&active)?;
            let inactive = self.inactive_doc_path(id);
            self.write_doc_at(&inactive, &doc)?;
            fs::remove_file(&active).map_err(|e| OrbitError::Io(e.to_string()))?;
            return Ok(true);
        }

        let inactive = self.inactive_doc_path(id);
        if inactive.exists() {
            let doc = self.read_doc_at(&inactive)?;
            self.write_doc_at(&inactive, &doc)?;
            return Ok(true);
        }

        Ok(false)
    }

    fn list_dir_docs(&self, dir: &Path, active: bool) -> Result<Vec<Activity>, OrbitError> {
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut paths = fs::read_dir(dir)
            .map_err(|e| OrbitError::Io(e.to_string()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| is_yaml(path))
            .collect::<Vec<_>>();
        paths.sort();

        let mut activities = Vec::new();
        for path in paths {
            activities.push(self.read_activity_at(&path, active)?);
        }
        Ok(activities)
    }

    fn read_doc_at(&self, path: &Path) -> Result<ActivityFileDocument, OrbitError> {
        let raw = fs::read_to_string(path).map_err(|e| OrbitError::Io(e.to_string()))?;
        serde_yaml::from_str::<ActivityFileDocument>(&raw).map_err(|e| {
            OrbitError::Store(format!("invalid activity file '{}': {e}", path.display()))
        })
    }

    fn read_activity_at(&self, path: &Path, is_active: bool) -> Result<Activity, OrbitError> {
        let doc = self.read_doc_at(path)?;
        let (created_at, updated_at) = file_timestamps(path)?;
        Ok(doc_to_work(doc, is_active, created_at, updated_at))
    }

    fn write_doc_at(&self, path: &Path, doc: &ActivityFileDocument) -> Result<(), OrbitError> {
        let yaml = serde_yaml::to_string(doc).map_err(|e| OrbitError::Store(e.to_string()))?;
        write_atomic(path, &yaml)
    }

    fn active_doc_path(&self, id: &str) -> PathBuf {
        self.active_dir().join(format!("{id}.yaml"))
    }

    fn inactive_doc_path(&self, id: &str) -> PathBuf {
        self.inactive_dir().join(format!("{id}.yaml"))
    }

    fn active_dir(&self) -> PathBuf {
        self.root.join("active")
    }

    fn inactive_dir(&self) -> PathBuf {
        self.root.join("inactive")
    }
}

fn doc_to_work(
    doc: ActivityFileDocument,
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
        workspace_path: doc.activity.workspace_path,
        created_by: doc.created_by,
        is_active,
        created_at,
        updated_at,
    }
}

fn file_timestamps(path: &Path) -> Result<(DateTime<Utc>, DateTime<Utc>), OrbitError> {
    let metadata = fs::metadata(path).map_err(|e| OrbitError::Io(e.to_string()))?;
    let created_at = metadata.created().ok().map(DateTime::<Utc>::from);
    let updated_at = metadata.modified().ok().map(DateTime::<Utc>::from);
    let now = Utc::now();

    let created_at = created_at.or(updated_at).unwrap_or(now);
    let updated_at = updated_at.unwrap_or(created_at);
    Ok((created_at, updated_at))
}

fn is_yaml(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml"))
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
