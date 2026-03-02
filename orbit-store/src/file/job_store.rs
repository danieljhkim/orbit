use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_types::{Job, OrbitError};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub(crate) struct JobFileStore {
    root: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct FileWorkInsert {
    pub id: String,
    pub spec_type: String,
    pub description: String,
    pub input_schema_json: serde_json::Value,
    pub output_schema_json: serde_json::Value,
    pub artifact_path_template: Option<String>,
    pub skill_refs: Vec<String>,
    pub identity_id: Option<String>,
    pub assigned_to: Option<String>,
    pub created_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobFileDocument {
    schema_version: u8,
    id: String,
    spec_type: String,
    description: String,
    input_schema_json: serde_json::Value,
    output_schema_json: serde_json::Value,
    artifact_path_template: Option<String>,
    skill_refs: Vec<String>,
    #[serde(default)]
    identity_id: Option<String>,
    #[serde(default)]
    assigned_to: Option<String>,
    #[serde(default)]
    created_by: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl JobFileStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn ensure_layout(&self) -> Result<(), OrbitError> {
        fs::create_dir_all(self.active_dir()).map_err(|e| OrbitError::Io(e.to_string()))?;
        fs::create_dir_all(self.inactive_dir()).map_err(|e| OrbitError::Io(e.to_string()))?;
        Ok(())
    }

    pub(crate) fn insert_work(&self, params: &FileWorkInsert) -> Result<Job, OrbitError> {
        self.ensure_layout()?;
        if self.get_job(&params.id)?.is_some() {
            return Err(OrbitError::InvalidInput(format!(
                "job already exists: {}",
                params.id
            )));
        }

        let now = Utc::now();
        let doc = JobFileDocument {
            schema_version: 1,
            id: params.id.clone(),
            spec_type: params.spec_type.clone(),
            description: params.description.clone(),
            input_schema_json: params.input_schema_json.clone(),
            output_schema_json: params.output_schema_json.clone(),
            artifact_path_template: params.artifact_path_template.clone(),
            skill_refs: params.skill_refs.clone(),
            identity_id: params.identity_id.clone(),
            assigned_to: params.assigned_to.clone(),
            created_by: params.created_by.clone(),
            created_at: now,
            updated_at: now,
        };
        self.write_doc_at(&self.active_doc_path(&doc.id), &doc)?;
        Ok(doc_to_work(doc, true))
    }

    pub(crate) fn list_jobs(&self, include_inactive: bool) -> Result<Vec<Job>, OrbitError> {
        let mut jobs = self.list_dir_docs(&self.active_dir(), true)?;
        if include_inactive {
            jobs.extend(self.list_dir_docs(&self.inactive_dir(), false)?);
        }
        jobs.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(jobs)
    }

    pub(crate) fn get_job(&self, id: &str) -> Result<Option<Job>, OrbitError> {
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

    pub(crate) fn disable_job(&self, id: &str) -> Result<bool, OrbitError> {
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

    fn list_dir_docs(&self, dir: &Path, active: bool) -> Result<Vec<Job>, OrbitError> {
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

        let mut jobs = Vec::new();
        for path in paths {
            let doc = self.read_doc_at(&path)?;
            jobs.push(doc_to_work(doc, active));
        }
        Ok(jobs)
    }

    fn read_doc_at(&self, path: &Path) -> Result<JobFileDocument, OrbitError> {
        let raw = fs::read_to_string(path).map_err(|e| OrbitError::Io(e.to_string()))?;
        serde_yaml::from_str::<JobFileDocument>(&raw)
            .map_err(|e| OrbitError::Store(format!("invalid job file '{}': {e}", path.display())))
    }

    fn write_doc_at(&self, path: &Path, doc: &JobFileDocument) -> Result<(), OrbitError> {
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

fn doc_to_work(doc: JobFileDocument, is_active: bool) -> Job {
    Job {
        id: doc.id,
        spec_type: doc.spec_type,
        description: doc.description,
        input_schema_json: doc.input_schema_json,
        output_schema_json: doc.output_schema_json,
        artifact_path_template: doc.artifact_path_template,
        skill_refs: doc.skill_refs,
        identity_id: doc.identity_id,
        assigned_to: doc.assigned_to,
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
