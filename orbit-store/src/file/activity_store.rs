use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_types::{Activity, OrbitError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone)]
pub(crate) struct ActivityFileStore {
    root: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct FileWorkInsert {
    pub id: String,
    pub spec_type: String,
    pub description: String,
    pub input_schema_json: Value,
    pub output_schema_json: Value,
    pub spec_config: Value,
    pub workspace_path: Option<String>,
    pub identity_id: Option<String>,
    pub created_by: Option<String>,
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
    #[serde(default)]
    identity_id: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
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

    pub(crate) fn insert_work(&self, params: &FileWorkInsert) -> Result<Activity, OrbitError> {
        self.ensure_layout()?;
        if self.get_activity(&params.id)?.is_some() {
            return Err(OrbitError::InvalidInput(format!(
                "activity already exists: {}",
                params.id
            )));
        }

        let now = Utc::now();
        let doc = ActivityFileDocument {
            schema_version: 1,
            created_by: params.created_by.clone(),
            identity_id: params.identity_id.clone(),
            created_at: now,
            updated_at: now,
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
                spec_config: params
                    .spec_config
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            },
        };
        self.write_doc_at(&self.active_doc_path(&doc.activity.id), &doc)?;
        Ok(doc_to_work(doc, true))
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
            let doc = self.read_doc_at(&active)?;
            return Ok(Some(doc_to_work(doc, true)));
        }
        let inactive = self.inactive_doc_path(id);
        if inactive.exists() {
            let doc = self.read_doc_at(&inactive)?;
            return Ok(Some(doc_to_work(doc, false)));
        }
        Ok(None)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update_activity(
        &self,
        id: &str,
        description: Option<String>,
        input_schema_json: Option<Value>,
        output_schema_json: Option<Value>,
        spec_config: Option<Value>,
        workspace_path: Option<Option<String>>,
        identity_id: Option<Option<String>>,
        is_active: Option<bool>,
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
        if let Some(v) = description {
            doc.activity.description = v;
        }
        if let Some(v) = input_schema_json {
            doc.activity.input_schema_json = normalize_json_schema_for_storage(v);
        }
        if let Some(v) = output_schema_json {
            doc.activity.output_schema_json = normalize_json_schema_for_storage(v);
        }
        if let Some(v) = spec_config {
            doc.activity.spec_config = v.as_object().cloned().unwrap_or_default();
        }
        if let Some(v) = workspace_path {
            doc.activity.workspace_path = v;
        }
        if let Some(v) = identity_id {
            doc.identity_id = v;
        }
        doc.updated_at = Utc::now();

        let new_active = is_active.unwrap_or(current_active);
        if new_active != current_active {
            // Move the file to the new location.
            let new_path = if new_active {
                self.active_doc_path(id)
            } else {
                self.inactive_doc_path(id)
            };
            self.write_doc_at(&new_path, &doc)?;
            fs::remove_file(&path).map_err(|e| OrbitError::Io(e.to_string()))?;
            return Ok(doc_to_work(doc, new_active));
        }
        self.write_doc_at(&path, &doc)?;
        Ok(doc_to_work(doc, new_active))
    }

    pub(crate) fn disable_activity(&self, id: &str) -> Result<bool, OrbitError> {
        self.ensure_layout()?;
        let active = self.active_doc_path(id);
        if active.exists() {
            let mut doc = self.read_doc_at(&active)?;
            doc.updated_at = Utc::now();
            let inactive = self.inactive_doc_path(id);
            self.write_doc_at(&inactive, &doc)?;
            fs::remove_file(&active).map_err(|e| OrbitError::Io(e.to_string()))?;
            return Ok(true);
        }

        let inactive = self.inactive_doc_path(id);
        if inactive.exists() {
            let mut doc = self.read_doc_at(&inactive)?;
            doc.updated_at = Utc::now();
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
            let doc = self.read_doc_at(&path)?;
            activities.push(doc_to_work(doc, active));
        }
        Ok(activities)
    }

    fn read_doc_at(&self, path: &Path) -> Result<ActivityFileDocument, OrbitError> {
        let raw = fs::read_to_string(path).map_err(|e| OrbitError::Io(e.to_string()))?;
        serde_yaml::from_str::<ActivityFileDocument>(&raw).map_err(|e| {
            OrbitError::Store(format!("invalid activity file '{}': {e}", path.display()))
        })
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

fn doc_to_work(doc: ActivityFileDocument, is_active: bool) -> Activity {
    Activity {
        id: doc.activity.id,
        spec_type: doc.activity.spec_type,
        description: doc.activity.description,
        input_schema_json: normalize_json_schema_for_runtime(doc.activity.input_schema_json),
        output_schema_json: normalize_json_schema_for_runtime(doc.activity.output_schema_json),
        spec_config: Value::Object(doc.activity.spec_config),
        workspace_path: doc.activity.workspace_path,
        identity_id: doc.identity_id,
        created_by: doc.created_by,
        is_active,
        created_at: doc.created_at,
        updated_at: doc.updated_at,
    }
}

fn write_atomic(path: &Path, content: &str) -> Result<(), OrbitError> {
    let parent = path.parent().ok_or_else(|| {
        OrbitError::Io(format!("cannot determine parent for '{}'", path.display()))
    })?;
    fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;

    let mut tmp = path.to_path_buf();
    let nanos = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    tmp.set_extension(format!("yaml.tmp.{nanos}"));
    fs::write(&tmp, content).map_err(|e| OrbitError::Io(e.to_string()))?;
    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(OrbitError::Io(err.to_string()));
    }
    Ok(())
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
