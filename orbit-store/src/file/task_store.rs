use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_types::{OrbitError, Task, TaskPriority, TaskStatus, TaskType};
use serde::{Deserialize, Serialize};

const TASK_DOC_FILE_NAME: &str = "task.yaml";
const DESCRIPTION_FILE_NAME: &str = "description.md";
const INSTRUCTIONS_FILE_NAME: &str = "instructions.md";
const EXECUTION_SUMMARY_FILE_NAME: &str = "execution-summary.md";
const ARTIFACTS_DIR_NAME: &str = "artifacts";
const TASK_SCHEMA_VERSION: u8 = 2;

#[derive(Clone)]
pub(crate) struct TaskFileStore {
    root: PathBuf,
}

#[derive(Clone)]
pub(crate) struct FileTaskInsert {
    pub title: String,
    pub description: String,
    pub instructions: String,
    pub execution_summary: String,
    pub context_files: Vec<String>,
    pub workspace_path: Option<String>,
    pub assigned_to: Option<String>,
    pub created_by: Option<String>,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub task_type: TaskType,
    pub branch: Option<String>,
    pub pr_number: Option<String>,
    pub proposed_by: Option<String>,
}

#[derive(Default, Clone)]
pub(crate) struct FileTaskUpdate {
    pub title: Option<String>,
    pub description: Option<String>,
    pub instructions: Option<String>,
    pub execution_summary: Option<String>,
    pub context_files: Option<Vec<String>>,
    pub workspace_path: Option<Option<String>>,
    pub assigned_to: Option<Option<String>>,
    pub created_by: Option<Option<String>>,
    pub status: Option<TaskStatus>,
    pub priority: Option<TaskPriority>,
    pub task_type: Option<TaskType>,
    pub branch: Option<Option<String>>,
    pub pr_number: Option<Option<String>>,
    pub proposed_by: Option<Option<String>>,
    pub proposal_approved_by: Option<Option<String>>,
    pub proposal_decision_note: Option<Option<String>>,
    pub review_approved_by: Option<Option<String>>,
    pub review_decision_note: Option<Option<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskStateDir {
    Proposed,
    Backlog,
    InProgress,
    Review,
    Done,
    Blocked,
    Archived,
}

impl TaskStateDir {
    fn as_dir(self) -> &'static str {
        match self {
            TaskStateDir::Proposed => "proposed",
            TaskStateDir::Backlog => "backlog",
            TaskStateDir::InProgress => "in_progress",
            TaskStateDir::Review => "review",
            TaskStateDir::Done => "done",
            TaskStateDir::Blocked => "blocked",
            TaskStateDir::Archived => "archived",
        }
    }

    fn to_status(self) -> TaskStatus {
        match self {
            TaskStateDir::Proposed => TaskStatus::Proposed,
            TaskStateDir::Backlog => TaskStatus::Backlog,
            TaskStateDir::InProgress => TaskStatus::InProgress,
            TaskStateDir::Review => TaskStatus::Review,
            TaskStateDir::Done => TaskStatus::Done,
            TaskStateDir::Blocked => TaskStatus::Blocked,
            TaskStateDir::Archived => TaskStatus::Archived,
        }
    }

    fn from_status(status: TaskStatus) -> Self {
        match status {
            TaskStatus::Proposed => TaskStateDir::Proposed,
            TaskStatus::Backlog => TaskStateDir::Backlog,
            TaskStatus::InProgress => TaskStateDir::InProgress,
            TaskStatus::Review => TaskStateDir::Review,
            TaskStatus::Done => TaskStateDir::Done,
            TaskStatus::Blocked => TaskStateDir::Blocked,
            TaskStatus::Archived => TaskStateDir::Archived,
        }
    }

    fn all() -> [TaskStateDir; 7] {
        [
            TaskStateDir::Proposed,
            TaskStateDir::Backlog,
            TaskStateDir::InProgress,
            TaskStateDir::Review,
            TaskStateDir::Done,
            TaskStateDir::Blocked,
            TaskStateDir::Archived,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskFileDocument {
    #[serde(rename = "schema_version", alias = "schemaVersion")]
    schema_version: u8,
    id: String,
    title: String,
    #[serde(default)]
    context_files: Vec<String>,
    #[serde(default)]
    workspace_path: Option<String>,
    #[serde(default)]
    assigned_to: Option<String>,
    #[serde(default)]
    created_by: Option<String>,
    priority: TaskPriority,
    #[serde(rename = "type", default = "default_task_type")]
    task_type: TaskType,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    pr_number: Option<String>,
    #[serde(default)]
    proposed_by: Option<String>,
    #[serde(default)]
    proposal_approved_by: Option<String>,
    #[serde(default)]
    proposal_decision_note: Option<String>,
    #[serde(default)]
    review_approved_by: Option<String>,
    #[serde(default)]
    review_decision_note: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    #[serde(default)]
    acceptance_criteria: Vec<String>,
    #[serde(default)]
    history: Vec<TaskHistoryEntry>,
    #[serde(default)]
    comments: Vec<TaskComment>,
    #[serde(default)]
    job_id: Option<String>,
    #[serde(default)]
    scheduler_id: Option<String>,
    #[serde(default)]
    scheduler_run_id: Option<String>,
}

#[derive(Debug, Clone)]
struct TaskBundle {
    doc: TaskFileDocument,
    description: String,
    instructions: String,
    execution_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TaskHistoryEntry {
    at: DateTime<Utc>,
    by: String,
    event: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TaskComment {
    at: DateTime<Utc>,
    by: String,
    message: String,
}

fn default_task_type() -> TaskType {
    TaskType::Task
}

impl TaskFileStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn ensure_layout(&self) -> Result<(), OrbitError> {
        for state in TaskStateDir::all() {
            fs::create_dir_all(self.root.join(state.as_dir()))
                .map_err(|e| OrbitError::Io(e.to_string()))?;
        }
        Ok(())
    }

    pub(crate) fn create_task(&self, params: FileTaskInsert) -> Result<Task, OrbitError> {
        self.ensure_layout()?;
        if params.title.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task title must not be empty".to_string(),
            ));
        }

        let now = Utc::now();
        let id = self.next_task_id(now)?;
        let history_actor = params
            .created_by
            .clone()
            .unwrap_or_else(|| "human".to_string());
        let initial_state = TaskStateDir::from_status(params.status);
        let bundle = TaskBundle {
            doc: TaskFileDocument {
                schema_version: TASK_SCHEMA_VERSION,
                id,
                title: params.title,
                context_files: params.context_files,
                workspace_path: params.workspace_path,
                assigned_to: params.assigned_to,
                created_by: params.created_by,
                priority: params.priority,
                task_type: params.task_type,
                branch: params.branch,
                pr_number: params.pr_number,
                proposed_by: params.proposed_by,
                proposal_approved_by: None,
                proposal_decision_note: None,
                review_approved_by: None,
                review_decision_note: None,
                created_at: now,
                updated_at: now,
                acceptance_criteria: Vec::new(),
                history: vec![TaskHistoryEntry {
                    at: now,
                    by: history_actor,
                    event: "created".to_string(),
                }],
                comments: Vec::new(),
                job_id: None,
                scheduler_id: None,
                scheduler_run_id: None,
            },
            description: params.description,
            instructions: params.instructions,
            execution_summary: params.execution_summary,
        };

        self.write_bundle_for_state(initial_state, &bundle)?;
        Ok(bundle_to_task(initial_state, bundle))
    }

    pub(crate) fn list_tasks(&self) -> Result<Vec<Task>, OrbitError> {
        let mut tasks = Vec::new();
        for state in TaskStateDir::all() {
            let dir = self.state_dir_path(state);
            if !dir.exists() {
                continue;
            }
            let mut task_dirs = fs::read_dir(&dir)
                .map_err(|e| OrbitError::Io(e.to_string()))?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect::<Vec<_>>();
            task_dirs.sort();

            for task_dir in task_dirs {
                let bundle = self.read_bundle_at(&task_dir)?;
                tasks.push(bundle_to_task(state, bundle));
            }
        }

        tasks.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(tasks)
    }

    pub(crate) fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
    ) -> Result<Vec<Task>, OrbitError> {
        let tasks = self.list_tasks()?;
        Ok(tasks
            .into_iter()
            .filter(|task| status.is_none_or(|value| task.status == value))
            .filter(|task| priority.is_none_or(|value| task.priority == value))
            .collect())
    }

    pub(crate) fn get_task(&self, id: &str) -> Result<Option<Task>, OrbitError> {
        let Some((state, task_dir)) = self.locate_task(id)? else {
            return Ok(None);
        };
        let bundle = self.read_bundle_at(&task_dir)?;
        Ok(Some(bundle_to_task(state, bundle)))
    }

    pub(crate) fn search_tasks(&self, query: &str) -> Result<Vec<Task>, OrbitError> {
        let lowered = query.to_lowercase();
        let tasks = self.list_tasks()?;
        Ok(tasks
            .into_iter()
            .filter(|task| {
                task.title.to_lowercase().contains(&lowered)
                    || task.description.to_lowercase().contains(&lowered)
            })
            .collect())
    }

    pub(crate) fn update_task(
        &self,
        id: &str,
        fields: &FileTaskUpdate,
    ) -> Result<Task, OrbitError> {
        let Some((current_state, current_dir)) = self.locate_task(id)? else {
            return Err(OrbitError::TaskNotFound(id.to_string()));
        };
        let mut bundle = self.read_bundle_at(&current_dir)?;

        if let Some(value) = &fields.title {
            bundle.doc.title = value.clone();
        }
        if let Some(value) = &fields.description {
            bundle.description = value.clone();
        }
        if let Some(value) = &fields.instructions {
            bundle.instructions = value.clone();
        }
        if let Some(value) = &fields.execution_summary {
            bundle.execution_summary = value.clone();
        }
        if let Some(value) = &fields.context_files {
            bundle.doc.context_files = value.clone();
        }
        if let Some(value) = &fields.workspace_path {
            bundle.doc.workspace_path = value.clone();
        }
        if let Some(value) = &fields.assigned_to {
            bundle.doc.assigned_to = value.clone();
        }
        if let Some(value) = &fields.created_by {
            bundle.doc.created_by = value.clone();
        }
        if let Some(value) = fields.priority {
            bundle.doc.priority = value;
        }
        if let Some(value) = fields.task_type {
            bundle.doc.task_type = value;
        }
        if let Some(value) = &fields.branch {
            bundle.doc.branch = value.clone();
        }
        if let Some(value) = &fields.pr_number {
            bundle.doc.pr_number = value.clone();
        }
        if let Some(value) = &fields.proposed_by {
            bundle.doc.proposed_by = value.clone();
        }
        if let Some(value) = &fields.proposal_approved_by {
            bundle.doc.proposal_approved_by = value.clone();
        }
        if let Some(value) = &fields.proposal_decision_note {
            bundle.doc.proposal_decision_note = value.clone();
        }
        if let Some(value) = &fields.review_approved_by {
            bundle.doc.review_approved_by = value.clone();
        }
        if let Some(value) = &fields.review_decision_note {
            bundle.doc.review_decision_note = value.clone();
        }

        let target_state = fields
            .status
            .map(TaskStateDir::from_status)
            .unwrap_or(current_state);

        let event = if target_state == current_state {
            None
        } else if target_state == TaskStateDir::Done {
            Some("completed".to_string())
        } else if target_state == TaskStateDir::Archived {
            Some("archived".to_string())
        } else {
            Some("moved".to_string())
        };

        bundle.doc.updated_at = Utc::now();
        if let Some(event) = event {
            bundle.doc.history.push(TaskHistoryEntry {
                at: bundle.doc.updated_at,
                by: "human".to_string(),
                event,
            });
        }

        if target_state == current_state {
            self.write_bundle_at(&current_dir, &bundle)?;
        } else {
            self.write_bundle_at(&current_dir, &bundle)?;
            let target_dir = self.task_dir(target_state, &bundle.doc.id);
            self.move_task_dir(&current_dir, &target_dir)?;
        }

        Ok(bundle_to_task(target_state, bundle))
    }

    pub(crate) fn delete_task(&self, id: &str) -> Result<bool, OrbitError> {
        let Some((_, task_dir)) = self.locate_task(id)? else {
            return Ok(false);
        };
        fs::remove_dir_all(task_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
        Ok(true)
    }

    fn next_task_id(&self, now: DateTime<Utc>) -> Result<String, OrbitError> {
        for attempt in 0..1024_u32 {
            let nanos = Utc::now().timestamp_nanos_opt().unwrap_or_default();
            let candidate = if attempt == 0 {
                format!("T{}-{nanos}", now.format("%Y%m%d-%H%M%S"))
            } else {
                format!("T{}-{nanos}-{attempt}", now.format("%Y%m%d-%H%M%S"))
            };
            if self.locate_task(&candidate)?.is_none() {
                return Ok(candidate);
            }
        }
        Err(OrbitError::Execution(
            "unable to allocate unique task id".to_string(),
        ))
    }

    fn locate_task(&self, id: &str) -> Result<Option<(TaskStateDir, PathBuf)>, OrbitError> {
        for state in TaskStateDir::all() {
            let task_dir = self.task_dir(state, id);
            if task_dir.is_dir() {
                return Ok(Some((state, task_dir)));
            }
        }
        Ok(None)
    }

    fn write_bundle_for_state(
        &self,
        state: TaskStateDir,
        bundle: &TaskBundle,
    ) -> Result<(), OrbitError> {
        self.write_bundle_at(&self.task_dir(state, &bundle.doc.id), bundle)
    }

    fn write_bundle_at(&self, task_dir: &Path, bundle: &TaskBundle) -> Result<(), OrbitError> {
        self.validate_bundle(bundle, Some(task_dir))?;
        fs::create_dir_all(task_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
        fs::create_dir_all(self.artifacts_dir(task_dir))
            .map_err(|e| OrbitError::Io(e.to_string()))?;

        atomic_write_string(
            &self.task_doc_path(task_dir),
            &serialize_task_doc_yaml(&bundle.doc)?,
        )?;
        atomic_write_string(&self.description_path(task_dir), &bundle.description)?;
        atomic_write_string(&self.instructions_path(task_dir), &bundle.instructions)?;
        atomic_write_string(
            &self.execution_summary_path(task_dir),
            &bundle.execution_summary,
        )?;
        Ok(())
    }

    fn read_bundle_at(&self, task_dir: &Path) -> Result<TaskBundle, OrbitError> {
        let doc_path = self.task_doc_path(task_dir);
        let raw = fs::read_to_string(&doc_path)
            .map_err(|e| bundle_read_error(&doc_path, "task metadata", e))?;
        let doc = serde_yaml::from_str::<TaskFileDocument>(&raw).map_err(|e| {
            OrbitError::Store(format!("invalid task file {}: {e}", doc_path.display()))
        })?;
        let bundle = TaskBundle {
            doc,
            description: read_required_text(&self.description_path(task_dir), "task description")?,
            instructions: read_required_text(
                &self.instructions_path(task_dir),
                "task instructions",
            )?,
            execution_summary: read_required_text(
                &self.execution_summary_path(task_dir),
                "task execution summary",
            )?,
        };
        self.validate_bundle(&bundle, Some(task_dir))?;
        Ok(bundle)
    }

    fn move_task_dir(&self, from: &Path, to: &Path) -> Result<(), OrbitError> {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;
        }
        fs::rename(from, to).map_err(|e| OrbitError::Io(e.to_string()))
    }

    fn validate_bundle(
        &self,
        bundle: &TaskBundle,
        task_dir: Option<&Path>,
    ) -> Result<(), OrbitError> {
        if bundle.doc.schema_version != TASK_SCHEMA_VERSION {
            return Err(OrbitError::InvalidInput(format!(
                "unsupported task schema version: {}",
                bundle.doc.schema_version
            )));
        }
        if bundle.doc.id.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task id must not be empty".to_string(),
            ));
        }
        if bundle.doc.title.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task title must not be empty".to_string(),
            ));
        }
        if let Some(task_dir) = task_dir {
            let Some(dir_name) = task_dir.file_name().and_then(|name| name.to_str()) else {
                return Err(OrbitError::Store(format!(
                    "invalid task directory path {}",
                    task_dir.display()
                )));
            };
            if dir_name != bundle.doc.id {
                return Err(OrbitError::Store(format!(
                    "task directory {} does not match task id {}",
                    task_dir.display(),
                    bundle.doc.id
                )));
            }
        }
        Ok(())
    }

    fn state_dir_path(&self, state: TaskStateDir) -> PathBuf {
        self.root.join(state.as_dir())
    }

    fn task_dir(&self, state: TaskStateDir, id: &str) -> PathBuf {
        self.state_dir_path(state).join(id)
    }

    fn task_doc_path(&self, task_dir: &Path) -> PathBuf {
        task_dir.join(TASK_DOC_FILE_NAME)
    }

    fn description_path(&self, task_dir: &Path) -> PathBuf {
        task_dir.join(DESCRIPTION_FILE_NAME)
    }

    fn instructions_path(&self, task_dir: &Path) -> PathBuf {
        task_dir.join(INSTRUCTIONS_FILE_NAME)
    }

    fn execution_summary_path(&self, task_dir: &Path) -> PathBuf {
        task_dir.join(EXECUTION_SUMMARY_FILE_NAME)
    }

    fn artifacts_dir(&self, task_dir: &Path) -> PathBuf {
        task_dir.join(ARTIFACTS_DIR_NAME)
    }
}

fn serialize_task_doc_yaml(doc: &TaskFileDocument) -> Result<String, OrbitError> {
    serde_yaml::to_string(doc).map_err(|e| OrbitError::Store(e.to_string()))
}

fn atomic_write_string(path: &Path, contents: &str) -> Result<(), OrbitError> {
    let parent = path
        .parent()
        .ok_or_else(|| OrbitError::Io(format!("path {} has no parent", path.display())))?;
    fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;

    let nanos = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| OrbitError::Io(format!("path {} has no file name", path.display())))?;
    let tmp_path = parent.join(format!(".{file_name}.tmp.{nanos}"));
    fs::write(&tmp_path, contents).map_err(|e| OrbitError::Io(e.to_string()))?;
    if let Err(err) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(OrbitError::Io(err.to_string()));
    }
    Ok(())
}

fn read_required_text(path: &Path, label: &str) -> Result<String, OrbitError> {
    fs::read_to_string(path).map_err(|e| bundle_read_error(path, label, e))
}

fn bundle_read_error(path: &Path, label: &str, err: std::io::Error) -> OrbitError {
    if err.kind() == std::io::ErrorKind::NotFound {
        OrbitError::Store(format!("missing {label} at {}", path.display()))
    } else {
        OrbitError::Io(err.to_string())
    }
}

fn bundle_to_task(state: TaskStateDir, bundle: TaskBundle) -> Task {
    Task {
        id: bundle.doc.id,
        title: bundle.doc.title,
        description: bundle.description,
        instructions: bundle.instructions,
        execution_summary: bundle.execution_summary,
        context_files: bundle.doc.context_files,
        workspace_path: bundle.doc.workspace_path,
        assigned_to: bundle.doc.assigned_to,
        created_by: bundle.doc.created_by,
        status: state.to_status(),
        priority: bundle.doc.priority,
        task_type: bundle.doc.task_type,
        branch: bundle.doc.branch,
        pr_number: bundle.doc.pr_number,
        proposed_by: bundle.doc.proposed_by,
        proposal_approved_by: bundle.doc.proposal_approved_by,
        proposal_decision_note: bundle.doc.proposal_decision_note,
        review_approved_by: bundle.doc.review_approved_by,
        review_decision_note: bundle.doc.review_decision_note,
        created_at: bundle.doc.created_at,
        updated_at: bundle.doc.updated_at,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{ARTIFACTS_DIR_NAME, EXECUTION_SUMMARY_FILE_NAME, FileTaskInsert, FileTaskUpdate};
    use super::{DESCRIPTION_FILE_NAME, INSTRUCTIONS_FILE_NAME, TASK_DOC_FILE_NAME, TaskFileStore};
    use orbit_types::{TaskPriority, TaskStatus, TaskType};
    use tempfile::tempdir;

    fn sample_insert(status: TaskStatus) -> FileTaskInsert {
        FileTaskInsert {
            title: "Bundle task".to_string(),
            description: "Task description".to_string(),
            instructions: "Task instructions".to_string(),
            execution_summary: String::new(),
            context_files: vec!["orbit-store/src/file/task_store.rs".to_string()],
            workspace_path: Some("/tmp/workspace".to_string()),
            assigned_to: Some("Codex".to_string()),
            created_by: Some("Codex".to_string()),
            status,
            priority: TaskPriority::High,
            task_type: TaskType::Refactor,
            branch: None,
            pr_number: None,
            proposed_by: Some("daniel".to_string()),
        }
    }

    #[test]
    fn create_task_persists_task_bundle_layout() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());

        let task = store
            .create_task(sample_insert(TaskStatus::Backlog))
            .expect("create task");
        let task_dir = dir.path().join("backlog").join(&task.id);

        assert!(task_dir.join(TASK_DOC_FILE_NAME).exists());
        assert!(task_dir.join(DESCRIPTION_FILE_NAME).exists());
        assert!(task_dir.join(INSTRUCTIONS_FILE_NAME).exists());
        assert!(task_dir.join(EXECUTION_SUMMARY_FILE_NAME).exists());
        assert!(task_dir.join(ARTIFACTS_DIR_NAME).is_dir());

        let yaml = fs::read_to_string(task_dir.join(TASK_DOC_FILE_NAME)).expect("read yaml");
        assert!(yaml.contains("schema_version: 2"));
        assert!(!yaml.contains("description:"));
        assert!(!yaml.contains("instructions:"));
        assert!(!yaml.contains("execution_summary:"));

        assert_eq!(
            fs::read_to_string(task_dir.join(DESCRIPTION_FILE_NAME)).expect("description"),
            "Task description"
        );
        assert_eq!(
            fs::read_to_string(task_dir.join(INSTRUCTIONS_FILE_NAME)).expect("instructions"),
            "Task instructions"
        );
    }

    #[test]
    fn update_task_rewrites_markdown_sidecars() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());

        let task = store
            .create_task(sample_insert(TaskStatus::Backlog))
            .expect("create task");
        let task_dir = dir.path().join("backlog").join(&task.id);

        let updated = store
            .update_task(
                &task.id,
                &FileTaskUpdate {
                    description: Some("Updated description".to_string()),
                    instructions: Some("Updated instructions".to_string()),
                    execution_summary: Some("Validated bundle layout".to_string()),
                    ..Default::default()
                },
            )
            .expect("update task");

        assert_eq!(updated.description, "Updated description");
        assert_eq!(updated.instructions, "Updated instructions");
        assert_eq!(updated.execution_summary, "Validated bundle layout");
        assert_eq!(
            fs::read_to_string(task_dir.join(DESCRIPTION_FILE_NAME)).expect("description"),
            "Updated description"
        );
        assert_eq!(
            fs::read_to_string(task_dir.join(INSTRUCTIONS_FILE_NAME)).expect("instructions"),
            "Updated instructions"
        );
        assert_eq!(
            fs::read_to_string(task_dir.join(EXECUTION_SUMMARY_FILE_NAME)).expect("summary"),
            "Validated bundle layout"
        );
    }

    #[test]
    fn status_transition_moves_task_directory_with_artifacts() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());

        let task = store
            .create_task(sample_insert(TaskStatus::Backlog))
            .expect("create task");
        let backlog_dir = dir.path().join("backlog").join(&task.id);
        let artifact_path = backlog_dir.join(ARTIFACTS_DIR_NAME).join("report.md");
        fs::write(&artifact_path, "# Report\n").expect("write artifact");

        let updated = store
            .update_task(
                &task.id,
                &FileTaskUpdate {
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .expect("move task");

        let in_progress_dir = dir.path().join("in_progress").join(&task.id);
        assert_eq!(updated.status, TaskStatus::InProgress);
        assert!(!backlog_dir.exists());
        assert!(in_progress_dir.exists());
        assert_eq!(
            fs::read_to_string(in_progress_dir.join(ARTIFACTS_DIR_NAME).join("report.md"))
                .expect("artifact"),
            "# Report\n"
        );
    }

    #[test]
    fn delete_task_removes_entire_task_directory() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());

        let task = store
            .create_task(sample_insert(TaskStatus::Backlog))
            .expect("create task");
        let task_dir = dir.path().join("backlog").join(&task.id);
        fs::write(
            task_dir.join(ARTIFACTS_DIR_NAME).join("note.md"),
            "artifact",
        )
        .expect("write artifact");

        assert!(store.delete_task(&task.id).expect("delete task"));
        assert!(!task_dir.exists());
    }

    #[test]
    fn list_and_search_tasks_read_markdown_sidecars() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());

        let one = store
            .create_task(sample_insert(TaskStatus::Backlog))
            .expect("create first task");
        let two = store
            .create_task(FileTaskInsert {
                title: "Another task".to_string(),
                description: "Searchable phrase".to_string(),
                instructions: "Other instructions".to_string(),
                execution_summary: String::new(),
                context_files: vec![],
                workspace_path: None,
                assigned_to: None,
                created_by: None,
                status: TaskStatus::Done,
                priority: TaskPriority::Medium,
                task_type: TaskType::Task,
                branch: None,
                pr_number: None,
                proposed_by: None,
            })
            .expect("create second task");

        let tasks = store.list_tasks().expect("list tasks");
        assert_eq!(tasks.len(), 2);
        assert!(tasks.iter().any(|task| task.id == one.id));
        assert!(tasks.iter().any(|task| task.id == two.id));

        let matches = store.search_tasks("searchable").expect("search tasks");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, two.id);
        assert_eq!(matches[0].description, "Searchable phrase");
    }

    #[test]
    fn get_task_errors_when_metadata_file_is_missing() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());
        let task_dir = dir.path().join("backlog").join("T-missing-doc");
        fs::create_dir_all(task_dir.join(ARTIFACTS_DIR_NAME)).expect("create task dir");
        fs::write(task_dir.join(DESCRIPTION_FILE_NAME), "desc").expect("write description");
        fs::write(task_dir.join(INSTRUCTIONS_FILE_NAME), "instructions")
            .expect("write instructions");
        fs::write(task_dir.join(EXECUTION_SUMMARY_FILE_NAME), "").expect("write summary");

        let err = store
            .get_task("T-missing-doc")
            .expect_err("missing metadata should error");
        assert!(err.to_string().contains("missing task metadata"));
    }

    #[test]
    fn get_task_errors_when_markdown_file_is_missing() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());

        let task = store
            .create_task(sample_insert(TaskStatus::Backlog))
            .expect("create task");
        let task_dir = dir.path().join("backlog").join(&task.id);
        fs::remove_file(task_dir.join(DESCRIPTION_FILE_NAME)).expect("remove description");

        let err = store
            .get_task(&task.id)
            .expect_err("missing description should error");
        assert!(err.to_string().contains("missing task description"));
    }

    #[test]
    fn get_task_errors_when_schema_version_is_invalid() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());

        let task = store
            .create_task(sample_insert(TaskStatus::Backlog))
            .expect("create task");
        let task_dir = dir.path().join("backlog").join(&task.id);
        let yaml_path = task_dir.join(TASK_DOC_FILE_NAME);
        let yaml = fs::read_to_string(&yaml_path).expect("read yaml");
        fs::write(
            &yaml_path,
            yaml.replace("schema_version: 2", "schema_version: 9"),
        )
        .expect("write yaml");

        let err = store
            .get_task(&task.id)
            .expect_err("invalid schema version should error");
        assert!(
            err.to_string()
                .contains("unsupported task schema version: 9")
        );
    }

    #[test]
    fn get_task_errors_when_title_is_empty() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());

        let task = store
            .create_task(sample_insert(TaskStatus::Backlog))
            .expect("create task");
        let task_dir = dir.path().join("backlog").join(&task.id);
        let yaml_path = task_dir.join(TASK_DOC_FILE_NAME);
        let yaml = fs::read_to_string(&yaml_path).expect("read yaml");
        fs::write(
            &yaml_path,
            yaml.replace("title: Bundle task", "title: \"\""),
        )
        .expect("write yaml");

        let err = store
            .get_task(&task.id)
            .expect_err("empty title should error");
        assert!(err.to_string().contains("task title must not be empty"));
    }
}
