use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_types::{
    ActorIdentity, OrbitError, OrbitId, Task, TaskComment, TaskComplexity, TaskHistoryEntry,
    TaskPriority, TaskStatus, TaskType,
};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value as YamlValue};

use crate::backend::{TaskCreateParams, TaskUpdateParams};
use crate::file::fs_utils::write_atomic;

const TASK_DOC_FILE_NAME: &str = "task.yaml";
const PLAN_FILE_NAME: &str = "plan.md";
const EXECUTION_SUMMARY_FILE_NAME: &str = "execution-summary.md";
const ARTIFACTS_DIR_NAME: &str = "artifacts";
const TASK_SCHEMA_VERSION: u8 = 4;

pub(crate) struct TaskFileStore {
    root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskStateDir {
    Proposed,
    Backlog,
    Someday,
    InProgress,
    Review,
    Done,
    Blocked,
    Archived,
    Rejected,
}

impl TaskStateDir {
    fn as_dir(self) -> &'static str {
        match self {
            TaskStateDir::Proposed => "proposed",
            TaskStateDir::Backlog => "backlog",
            TaskStateDir::Someday => "someday",
            TaskStateDir::InProgress => "in_progress",
            TaskStateDir::Review => "review",
            TaskStateDir::Done => "done",
            TaskStateDir::Blocked => "blocked",
            TaskStateDir::Archived => "archived",
            TaskStateDir::Rejected => "rejected",
        }
    }

    fn to_status(self) -> TaskStatus {
        match self {
            TaskStateDir::Proposed => TaskStatus::Proposed,
            TaskStateDir::Backlog => TaskStatus::Backlog,
            TaskStateDir::Someday => TaskStatus::Someday,
            TaskStateDir::InProgress => TaskStatus::InProgress,
            TaskStateDir::Review => TaskStatus::Review,
            TaskStateDir::Done => TaskStatus::Done,
            TaskStateDir::Blocked => TaskStatus::Blocked,
            TaskStateDir::Archived => TaskStatus::Archived,
            TaskStateDir::Rejected => TaskStatus::Rejected,
        }
    }

    fn from_status(status: TaskStatus) -> Self {
        match status {
            TaskStatus::Proposed => TaskStateDir::Proposed,
            TaskStatus::Backlog => TaskStateDir::Backlog,
            TaskStatus::Someday => TaskStateDir::Someday,
            TaskStatus::InProgress => TaskStateDir::InProgress,
            TaskStatus::Review => TaskStateDir::Review,
            TaskStatus::Done => TaskStateDir::Done,
            TaskStatus::Blocked => TaskStateDir::Blocked,
            TaskStatus::Archived => TaskStateDir::Archived,
            TaskStatus::Rejected => TaskStateDir::Rejected,
        }
    }

    fn all() -> [TaskStateDir; 9] {
        [
            TaskStateDir::Proposed,
            TaskStateDir::Backlog,
            TaskStateDir::Someday,
            TaskStateDir::InProgress,
            TaskStateDir::Review,
            TaskStateDir::Done,
            TaskStateDir::Blocked,
            TaskStateDir::Archived,
            TaskStateDir::Rejected,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskFileDocument {
    #[serde(rename = "schema_version")]
    schema_version: u8,
    id: String,
    #[serde(default)]
    parent_id: Option<OrbitId>,
    #[serde(rename = "type", default = "default_task_type")]
    task_type: TaskType,
    priority: TaskPriority,
    #[serde(default)]
    complexity: Option<TaskComplexity>,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    acceptance_criteria: Vec<String>,
    #[serde(default)]
    context_files: Vec<String>,
    #[serde(default)]
    workspace_path: Option<String>,
    #[serde(default)]
    repo_root: Option<String>,
    #[serde(default)]
    created_by: Option<String>,
    #[serde(default)]
    actor_identity: ActorIdentity,
    /// Legacy field — kept for deserialization of existing YAML files only.
    /// Never written back; `actor_identity` is the source of truth.
    #[serde(default, skip_serializing)]
    agent: Option<String>,
    /// Legacy field — kept for deserialization of existing YAML files only.
    /// Never written back; `actor_identity` is the source of truth.
    #[serde(default, skip_serializing)]
    model: Option<String>,
    #[serde(default)]
    assigned_to: Option<String>,
    #[serde(default)]
    proposed_by: Option<String>,
    #[serde(default)]
    pr_number: Option<String>,
    #[serde(default)]
    source_task_id: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    #[serde(default)]
    history: Vec<TaskHistoryEntry>,
    #[serde(default)]
    comments: Vec<TaskComment>,
}

#[derive(Debug, Clone)]
struct TaskBundle {
    doc: TaskFileDocument,
    plan: String,
    execution_summary: String,
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

    pub(crate) fn create_task(&self, params: TaskCreateParams) -> Result<Task, OrbitError> {
        self.ensure_layout()?;
        if params.title.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task title must not be empty".to_string(),
            ));
        }
        if params.actor.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task actor must not be empty".to_string(),
            ));
        }

        let now = Utc::now();
        let id = self.next_task_id(now)?;
        let initial_state = TaskStateDir::from_status(params.status);
        let bundle = TaskBundle {
            doc: TaskFileDocument {
                schema_version: TASK_SCHEMA_VERSION,
                id,
                parent_id: params.parent_id,
                title: params.title,
                description: params.description,
                context_files: params.context_files,
                workspace_path: params.workspace_path,
                repo_root: params.repo_root,
                assigned_to: params.assigned_to,
                created_by: params.created_by,
                actor_identity: params.actor_identity,
                agent: None,
                model: None,
                priority: params.priority,
                complexity: params.complexity,
                task_type: params.task_type,
                pr_number: params.pr_number,
                proposed_by: params.proposed_by,
                source_task_id: params.source_task_id,
                created_at: now,
                updated_at: now,
                acceptance_criteria: Vec::new(),
                history: vec![TaskHistoryEntry {
                    at: now,
                    by: params.actor,
                    event: "created".to_string(),
                    note: None,
                    from_status: None,
                    to_status: Some(params.status),
                }],
                comments: params.comments,
            },
            plan: params.plan,
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
        parent_id: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        let tasks = self.list_tasks()?;
        Ok(tasks
            .into_iter()
            .filter(|task| status.is_none_or(|value| task.status == value))
            .filter(|task| priority.is_none_or(|value| task.priority == value))
            .filter(|task| parent_id.is_none_or(|value| task.parent_id.as_deref() == Some(value)))
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
        fields: &TaskUpdateParams,
    ) -> Result<Task, OrbitError> {
        if fields.actor.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task actor must not be empty".to_string(),
            ));
        }
        let Some((current_state, current_dir)) = self.locate_task(id)? else {
            return Err(OrbitError::TaskNotFound(id.to_string()));
        };
        let mut bundle = self.read_bundle_at(&current_dir)?;

        let title_changed = if let Some(value) = &fields.title {
            let changed = *value != bundle.doc.title;
            bundle.doc.title = value.clone();
            changed
        } else {
            false
        };
        if let Some(value) = &fields.description {
            bundle.doc.description = value.clone();
        }
        if let Some(value) = &fields.plan {
            bundle.plan = value.clone();
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
        if let Some(value) = &fields.repo_root {
            bundle.doc.repo_root = value.clone();
        }
        if let Some(value) = &fields.assigned_to {
            bundle.doc.assigned_to = value.clone();
        }
        if let Some(value) = &fields.created_by {
            bundle.doc.created_by = value.clone();
        }
        if let Some(ref identity) = fields.actor_identity {
            bundle.doc.actor_identity = identity.clone();
        }
        if let Some(value) = fields.priority {
            bundle.doc.priority = value;
        }
        if let Some(value) = fields.complexity {
            bundle.doc.complexity = Some(value);
        }
        if let Some(value) = fields.task_type {
            bundle.doc.task_type = value;
        }
        if let Some(value) = &fields.pr_number {
            bundle.doc.pr_number = value.clone();
        }
        if let Some(value) = &fields.proposed_by {
            bundle.doc.proposed_by = value.clone();
        }
        if let Some(value) = &fields.source_task_id {
            bundle.doc.source_task_id = value.clone();
        }
        if !fields.append_history.is_empty() {
            bundle.doc.history.extend(fields.append_history.clone());
        }
        if !fields.append_comments.is_empty() {
            bundle.doc.comments.extend(fields.append_comments.clone());
        }

        let target_state = fields
            .status
            .map(TaskStateDir::from_status)
            .unwrap_or(current_state);
        let status_transition = (target_state != current_state)
            .then_some((current_state.to_status(), target_state.to_status()));

        let event = if let Some(event) = fields.status_event.clone() {
            Some(event)
        } else if target_state == current_state {
            None
        } else {
            Some("status_changed".to_string())
        };

        bundle.doc.updated_at = Utc::now();
        if let Some(event) = event {
            bundle.doc.history.push(TaskHistoryEntry {
                at: bundle.doc.updated_at,
                by: fields.actor.clone(),
                event,
                note: fields.status_note.clone(),
                from_status: status_transition.map(|(from, _)| from),
                to_status: status_transition.map(|(_, to)| to),
            });
        }
        if title_changed {
            bundle.doc.history.push(TaskHistoryEntry {
                at: bundle.doc.updated_at,
                by: fields.actor.clone(),
                event: "renamed".to_string(),
                note: None,
                from_status: None,
                to_status: None,
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
        let base = format!("T{}", now.format("%Y%m%d-%H%M%S"));
        if self.locate_task(&base)?.is_none() {
            return Ok(base);
        }
        for suffix in 2..1024_u32 {
            let candidate = format!("{base}-{suffix}");
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
        let mut bundle = bundle.clone();
        bundle.doc.schema_version = TASK_SCHEMA_VERSION;
        self.validate_bundle(&bundle, Some(task_dir))?;
        fs::create_dir_all(task_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
        fs::create_dir_all(self.artifacts_dir(task_dir))
            .map_err(|e| OrbitError::Io(e.to_string()))?;

        write_atomic(
            &self.task_doc_path(task_dir),
            &serialize_task_doc_yaml(&bundle.doc)?,
        )?;
        write_atomic(&self.plan_path(task_dir), &bundle.plan)?;
        write_atomic(
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
            plan: read_required_text(&self.plan_path(task_dir), "task plan")?,
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

    fn plan_path(&self, task_dir: &Path) -> PathBuf {
        task_dir.join(PLAN_FILE_NAME)
    }

    fn execution_summary_path(&self, task_dir: &Path) -> PathBuf {
        task_dir.join(EXECUTION_SUMMARY_FILE_NAME)
    }

    fn artifacts_dir(&self, task_dir: &Path) -> PathBuf {
        task_dir.join(ARTIFACTS_DIR_NAME)
    }
}

fn serialize_task_doc_yaml(doc: &TaskFileDocument) -> Result<String, OrbitError> {
    let mut yaml = String::new();
    yaml.push_str(&yaml_field("schema_version", &doc.schema_version)?);

    yaml.push_str(&yaml_section("identity"));
    yaml.push_str(&yaml_field("id", &doc.id)?);
    if let Some(ref parent_id) = doc.parent_id {
        yaml.push_str(&yaml_field("parent_id", parent_id)?);
    }
    yaml.push_str(&yaml_field("type", &doc.task_type)?);
    yaml.push_str(&yaml_field("priority", &doc.priority)?);
    if let Some(complexity) = doc.complexity {
        yaml.push_str(&yaml_field("complexity", &complexity)?);
    }

    yaml.push_str(&yaml_section("content"));
    yaml.push_str(&yaml_field("title", &doc.title)?);
    yaml.push_str(&yaml_field("description", &doc.description)?);
    yaml.push_str(&yaml_field(
        "acceptance_criteria",
        &doc.acceptance_criteria,
    )?);

    yaml.push_str(&yaml_section("context"));
    yaml.push_str(&yaml_field("context_files", &doc.context_files)?);
    yaml.push_str(&yaml_field("workspace_path", &doc.workspace_path)?);
    yaml.push_str(&yaml_field("repo_root", &doc.repo_root)?);

    yaml.push_str(&yaml_section("ownership"));
    yaml.push_str(&yaml_field("created_by", &doc.created_by)?);
    yaml.push_str(&yaml_field("assigned_to", &doc.assigned_to)?);

    yaml.push_str(&yaml_section("proposal workflow"));
    yaml.push_str(&yaml_field("proposed_by", &doc.proposed_by)?);

    yaml.push_str(&yaml_section("implementation"));
    yaml.push_str(&yaml_field("actor_identity", &doc.actor_identity)?);
    yaml.push_str(&yaml_field("pr_number", &doc.pr_number)?);

    if doc.source_task_id.is_some() {
        yaml.push_str(&yaml_section("attribution"));
        yaml.push_str(&yaml_field("source_task_id", &doc.source_task_id)?);
    }

    yaml.push_str(&yaml_section("timestamps"));
    yaml.push_str(&yaml_field("created_at", &doc.created_at)?);
    yaml.push_str(&yaml_field("updated_at", &doc.updated_at)?);

    yaml.push_str(&yaml_section("audit trail"));
    yaml.push_str(&yaml_field("history", &doc.history)?);
    yaml.push_str(&yaml_field("comments", &doc.comments)?);

    Ok(yaml)
}

fn yaml_section(name: &str) -> String {
    format!("\n# ---- {name} ----\n")
}

fn yaml_field(key: &str, value: &impl Serialize) -> Result<String, OrbitError> {
    let mut mapping = Mapping::new();
    mapping.insert(
        YamlValue::String(key.to_string()),
        serde_yaml::to_value(value).map_err(|e| OrbitError::Store(e.to_string()))?,
    );
    serde_yaml::to_string(&mapping).map_err(|e| OrbitError::Store(e.to_string()))
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
    // When loading from YAML, if actor_identity is System (the default) but
    // legacy agent/model fields are present, reconstruct the identity from them.
    let actor_identity = if bundle.doc.actor_identity.is_system()
        && bundle.doc.agent.is_some()
    {
        ActorIdentity::from_legacy(
            bundle.doc.agent.as_deref(),
            bundle.doc.model.as_deref(),
        )
    } else {
        bundle.doc.actor_identity
    };

    Task {
        id: bundle.doc.id,
        parent_id: bundle.doc.parent_id,
        title: bundle.doc.title,
        description: bundle.doc.description,
        plan: bundle.plan,
        execution_summary: bundle.execution_summary,
        context_files: bundle.doc.context_files,
        workspace_path: bundle.doc.workspace_path,
        repo_root: bundle.doc.repo_root,
        assigned_to: bundle.doc.assigned_to,
        created_by: bundle.doc.created_by,
        actor_identity,
        status: state.to_status(),
        priority: bundle.doc.priority,
        complexity: bundle.doc.complexity,
        task_type: bundle.doc.task_type,
        pr_number: bundle.doc.pr_number,
        proposed_by: bundle.doc.proposed_by,
        source_task_id: bundle.doc.source_task_id,
        comments: bundle.doc.comments,
        history: bundle.doc.history,
        created_at: bundle.doc.created_at,
        updated_at: bundle.doc.updated_at,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{ARTIFACTS_DIR_NAME, EXECUTION_SUMMARY_FILE_NAME};
    use super::{PLAN_FILE_NAME, TASK_DOC_FILE_NAME, TaskFileStore};
    use crate::backend::{TaskCreateParams, TaskUpdateParams};
    use chrono::Utc;
    use orbit_types::{ActorIdentity, TaskComment, TaskComplexity, TaskPriority, TaskStatus, TaskType};
    use tempfile::tempdir;

    fn sample_insert(status: TaskStatus) -> TaskCreateParams {
        TaskCreateParams {
            actor: "Codex".to_string(),
            parent_id: None,
            title: "Bundle task".to_string(),
            description: "Task description".to_string(),
            plan: "Task plan".to_string(),
            execution_summary: String::new(),
            context_files: vec!["orbit-store/src/file/task_store.rs".to_string()],
            workspace_path: None,
            repo_root: Some("/tmp/repo".to_string()),
            assigned_to: Some("Codex".to_string()),
            created_by: Some("Codex".to_string()),
            actor_identity: ActorIdentity::System,
            status,
            priority: TaskPriority::High,
            complexity: Some(TaskComplexity::Medium),
            task_type: TaskType::Refactor,
            pr_number: None,
            proposed_by: Some("daniel".to_string()),
            source_task_id: None,
            comments: Vec::new(),
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
        assert!(task_dir.join(PLAN_FILE_NAME).exists());
        assert!(task_dir.join(EXECUTION_SUMMARY_FILE_NAME).exists());
        assert!(task_dir.join(ARTIFACTS_DIR_NAME).is_dir());

        let yaml = fs::read_to_string(task_dir.join(TASK_DOC_FILE_NAME)).expect("read yaml");
        assert!(yaml.contains("schema_version: 4"));
        assert!(!yaml.contains("parent_id:"));
        assert!(yaml.contains("description: Task description"));
        assert!(yaml.contains("context_files:"));
        assert!(yaml.contains("created_by: Codex"));
        assert!(yaml.contains("assigned_to: Codex"));
        assert!(yaml.contains("proposed_by: daniel"));
        assert!(yaml.contains("created_at:"));
        assert!(yaml.contains("updated_at:"));
        assert!(!yaml.contains("contextFiles:"));
        assert!(!yaml.contains("workspacePath:"));
        assert!(!yaml.contains("createdBy:"));
        assert!(!yaml.contains("assignedTo:"));
        assert!(!yaml.contains("proposedBy:"));
        assert!(!yaml.contains("createdAt:"));
        assert!(!yaml.contains("updatedAt:"));
        assert!(!yaml.contains("plan:"));
        assert!(!yaml.contains("execution_summary:"));
        assert_eq!(
            fs::read_to_string(task_dir.join(PLAN_FILE_NAME)).expect("plan"),
            "Task plan"
        );
    }

    #[test]
    fn update_task_rewrites_task_yaml_and_plan_sidecar() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());

        let task = store
            .create_task(sample_insert(TaskStatus::Backlog))
            .expect("create task");
        let task_dir = dir.path().join("backlog").join(&task.id);

        let updated = store
            .update_task(
                &task.id,
                &TaskUpdateParams {
                    actor: "Codex".to_string(),
                    description: Some("Updated description".to_string()),
                    plan: Some("Updated plan".to_string()),
                    execution_summary: Some("Validated bundle layout".to_string()),
                    actor_identity: Some(ActorIdentity::agent("codex", "gpt-5.4")),
                    ..Default::default()
                },
            )
            .expect("update task");

        assert_eq!(updated.description, "Updated description");
        assert_eq!(updated.plan, "Updated plan");
        assert_eq!(updated.execution_summary, "Validated bundle layout");
        assert_eq!(updated.actor_identity, ActorIdentity::agent("codex", "gpt-5.4"));
        let yaml = fs::read_to_string(task_dir.join(TASK_DOC_FILE_NAME)).expect("read yaml");
        assert!(yaml.contains("schema_version: 4"));
        assert!(yaml.contains("description: Updated description"));
        assert!(yaml.contains("updated_at:"));
        assert!(!yaml.contains("updatedAt:"));
        assert_eq!(
            fs::read_to_string(task_dir.join(PLAN_FILE_NAME)).expect("plan"),
            "Updated plan"
        );
        assert_eq!(
            fs::read_to_string(task_dir.join(EXECUTION_SUMMARY_FILE_NAME)).expect("summary"),
            "Validated bundle layout"
        );
    }

    #[test]
    fn get_task_errors_when_legacy_decision_fields_are_present() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());

        let task = store
            .create_task(sample_insert(TaskStatus::Backlog))
            .expect("create task");
        let task_yaml_path = dir
            .path()
            .join("backlog")
            .join(&task.id)
            .join(TASK_DOC_FILE_NAME);
        let task_yaml = fs::read_to_string(&task_yaml_path).expect("read yaml");
        let task_yaml = task_yaml.replace(
            "proposed_by: daniel\n",
            "proposed_by: daniel\nproposal_approved_by: reviewer\nproposal_decision_note: ship it\n",
        );
        fs::write(&task_yaml_path, task_yaml).expect("write yaml");

        let err = store
            .get_task(&task.id)
            .expect_err("legacy fields should be rejected");
        assert!(err.to_string().contains("proposal_approved_by"));
    }

    #[test]
    fn task_yaml_contains_section_comments_in_order() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());

        let task = store
            .create_task(sample_insert(TaskStatus::Backlog))
            .expect("create task");
        let yaml = fs::read_to_string(
            dir.path()
                .join("backlog")
                .join(&task.id)
                .join(TASK_DOC_FILE_NAME),
        )
        .expect("read yaml");

        let identity_idx = yaml.find("# ---- identity ----").expect("identity section");
        let id_idx = yaml.find("id: ").expect("id field");
        let content_idx = yaml.find("# ---- content ----").expect("content section");
        let title_idx = yaml.find("title: ").expect("title field");
        let audit_idx = yaml.find("# ---- audit trail ----").expect("audit section");
        let history_idx = yaml.find("history:").expect("history field");

        assert!(
            identity_idx < id_idx,
            "identity section should precede id field"
        );
        assert!(
            content_idx < title_idx,
            "content section should precede title field"
        );
        assert!(
            audit_idx < history_idx,
            "audit section should precede history field"
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
                &TaskUpdateParams {
                    actor: "daniel".to_string(),
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .expect("move task");

        let in_progress_dir = dir.path().join("in_progress").join(&task.id);
        assert_eq!(updated.status, TaskStatus::InProgress);
        assert!(!backlog_dir.exists());
        assert!(in_progress_dir.exists());
        let yaml = fs::read_to_string(in_progress_dir.join(TASK_DOC_FILE_NAME)).expect("read yaml");
        assert!(yaml.contains("by: daniel"));
        assert!(yaml.contains("event: status_changed"));
        assert!(yaml.contains("from_status: backlog"));
        assert!(yaml.contains("to_status: in_progress"));
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
    fn list_and_search_tasks_read_task_bundle_content() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());

        let one = store
            .create_task(sample_insert(TaskStatus::Backlog))
            .expect("create first task");
        let two = store
            .create_task(TaskCreateParams {
                actor: "Codex".to_string(),
                parent_id: None,
                title: "Another task".to_string(),
                description: "Searchable phrase".to_string(),
                plan: "Other plan".to_string(),
                execution_summary: String::new(),
                context_files: vec![],
                workspace_path: None,
                repo_root: None,
                assigned_to: None,
                created_by: None,
                actor_identity: ActorIdentity::System,
                status: TaskStatus::Done,
                priority: TaskPriority::Medium,
                complexity: None,
                task_type: TaskType::Task,
                pr_number: None,
                proposed_by: None,
                source_task_id: None,
                comments: Vec::new(),
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
    fn create_task_persists_parent_id_when_present() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());

        let created = store
            .create_task(TaskCreateParams {
                actor: "Codex".to_string(),
                parent_id: Some("T20260320-000001".to_string()),
                title: "Nested task".to_string(),
                description: "desc".to_string(),
                plan: "plan".to_string(),
                execution_summary: String::new(),
                context_files: vec![],
                workspace_path: None,
                repo_root: None,
                assigned_to: None,
                created_by: None,
                actor_identity: ActorIdentity::System,
                status: TaskStatus::Backlog,
                priority: TaskPriority::Medium,
                complexity: None,
                task_type: TaskType::Task,
                pr_number: None,
                proposed_by: None,
                source_task_id: None,
                comments: Vec::new(),
            })
            .expect("create nested task");

        assert_eq!(created.parent_id.as_deref(), Some("T20260320-000001"));
        let yaml = fs::read_to_string(
            dir.path()
                .join("backlog")
                .join(&created.id)
                .join(TASK_DOC_FILE_NAME),
        )
        .expect("read yaml");
        assert!(yaml.contains("parent_id: T20260320-000001"));
    }

    #[test]
    fn list_tasks_filtered_by_parent_id_returns_only_matching_subtasks() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());

        let parent = store
            .create_task(TaskCreateParams {
                title: "Parent".to_string(),
                ..sample_insert(TaskStatus::Backlog)
            })
            .expect("create parent");

        let child = store
            .create_task(TaskCreateParams {
                parent_id: Some(parent.id.clone()),
                title: "Child".to_string(),
                ..sample_insert(TaskStatus::Backlog)
            })
            .expect("create child");

        store
            .create_task(TaskCreateParams {
                title: "Unrelated".to_string(),
                ..sample_insert(TaskStatus::Backlog)
            })
            .expect("create unrelated");

        let subtasks = store
            .list_tasks_filtered(None, None, Some(&parent.id))
            .expect("filter by parent");

        assert_eq!(subtasks.len(), 1);
        assert_eq!(subtasks[0].id, child.id);
        assert_eq!(subtasks[0].parent_id.as_deref(), Some(parent.id.as_str()));
    }

    #[test]
    fn get_task_errors_when_metadata_file_is_missing() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());
        let task_dir = dir.path().join("backlog").join("T-missing-doc");
        fs::create_dir_all(task_dir.join(ARTIFACTS_DIR_NAME)).expect("create task dir");
        fs::write(task_dir.join(PLAN_FILE_NAME), "plan").expect("write plan");
        fs::write(task_dir.join(EXECUTION_SUMMARY_FILE_NAME), "").expect("write summary");

        let err = store
            .get_task("T-missing-doc")
            .expect_err("missing metadata should error");
        assert!(err.to_string().contains("missing task metadata"));
    }

    #[test]
    fn get_task_errors_when_plan_file_is_missing() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());

        let task = store
            .create_task(sample_insert(TaskStatus::Backlog))
            .expect("create task");
        let task_dir = dir.path().join("backlog").join(&task.id);
        fs::remove_file(task_dir.join(PLAN_FILE_NAME)).expect("remove plan");

        let err = store
            .get_task(&task.id)
            .expect_err("missing plan should error");
        assert!(err.to_string().contains("missing task plan"));
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
            yaml.replace("schema_version: 4", "schema_version: 9"),
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
    fn create_task_persists_complexity_when_present() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());

        let task = store
            .create_task(sample_insert(TaskStatus::Backlog))
            .expect("create task");
        let task_dir = dir.path().join("backlog").join(&task.id);
        let yaml = fs::read_to_string(task_dir.join(TASK_DOC_FILE_NAME)).expect("read yaml");

        assert!(yaml.contains("complexity: medium"));
        assert_eq!(task.complexity, Some(TaskComplexity::Medium));
    }

    #[test]
    fn get_task_loads_legacy_history_entries_without_transition_fields() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());

        let task = store
            .create_task(sample_insert(TaskStatus::Backlog))
            .expect("create task");
        let yaml_path = dir
            .path()
            .join("backlog")
            .join(&task.id)
            .join(TASK_DOC_FILE_NAME);
        let yaml = fs::read_to_string(&yaml_path).expect("read yaml");
        fs::write(
            &yaml_path,
            yaml.replace(
                "event: created\n  note: null\n  to_status: backlog\n",
                "event: moved\n  note: null\n",
            ),
        )
        .expect("write yaml");

        let loaded = store.get_task(&task.id).expect("load task").expect("task");
        assert_eq!(loaded.history.len(), 1);
        assert_eq!(loaded.history[0].event, "moved");
        assert_eq!(loaded.history[0].from_status, None);
        assert_eq!(loaded.history[0].to_status, None);
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

    #[test]
    fn update_task_appends_comments_without_replacing_existing_entries() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());

        let task = store
            .create_task(TaskCreateParams {
                comments: vec![TaskComment {
                    at: Utc::now(),
                    by: "creator".to_string(),
                    message: "created with context".to_string(),
                }],
                ..sample_insert(TaskStatus::Backlog)
            })
            .expect("create task");

        let updated = store
            .update_task(
                &task.id,
                &TaskUpdateParams {
                    actor: "Codex".to_string(),
                    append_comments: vec![TaskComment {
                        at: Utc::now(),
                        by: "reviewer".to_string(),
                        message: "needs follow-up".to_string(),
                    }],
                    ..Default::default()
                },
            )
            .expect("append comment");

        assert_eq!(updated.comments.len(), 2);
        assert_eq!(updated.comments[0].by, "creator");
        assert_eq!(updated.comments[1].by, "reviewer");
        assert_eq!(updated.comments[1].message, "needs follow-up");
    }

    #[test]
    fn task_id_uses_datetime_format_without_nanosecond_suffix() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());

        let task = store
            .create_task(sample_insert(TaskStatus::Backlog))
            .expect("create task");

        // ID must be exactly T<YYYYMMDD>-<HHMMSS> (16 chars) with no
        // nanosecond suffix. Conflict suffix (-2, -3, …) is only added on collision.
        assert_eq!(
            task.id.len(),
            16,
            "task id '{}' must be 16 chars (T+8date+dash+6time), got {}",
            task.id,
            task.id.len()
        );
        assert!(task.id.starts_with('T'), "must start with T");
        let (date, time) = task.id[1..].split_once('-').expect("has dash");
        assert_eq!(date.len(), 8, "date part must be 8 digits");
        assert!(
            date.chars().all(|c| c.is_ascii_digit()),
            "date must be numeric"
        );
        assert_eq!(time.len(), 6, "time part must be 6 digits");
        assert!(
            time.chars().all(|c| c.is_ascii_digit()),
            "time must be numeric"
        );
    }

    #[test]
    fn task_id_conflict_appends_numeric_suffix() {
        let dir = tempdir().expect("tempdir");
        let store = TaskFileStore::new(dir.path().to_path_buf());

        // Force a collision by pre-creating a task directory with the expected id.
        let now = Utc::now();
        let base_id = format!("T{}", now.format("%Y%m%d-%H%M%S"));
        let occupied = dir.path().join("backlog").join(&base_id);
        std::fs::create_dir_all(occupied.join("artifacts")).expect("create occupied dir");
        std::fs::write(occupied.join("task.yaml"), "").expect("placeholder");
        std::fs::write(occupied.join("plan.md"), "").expect("placeholder");
        std::fs::write(occupied.join("execution-summary.md"), "").expect("placeholder");

        let task = store
            .create_task(sample_insert(TaskStatus::Backlog))
            .expect("create task with suffix");

        assert!(
            task.id.starts_with(&base_id),
            "id '{}' must start with '{base_id}'",
            task.id
        );
        assert_ne!(task.id, base_id, "id must differ from occupied base id");
        // Suffix must be a dash followed by a small integer.
        let suffix = task.id.strip_prefix(&base_id).expect("has prefix");
        assert!(
            suffix.starts_with('-'),
            "suffix '{suffix}' must start with '-'"
        );
        let num: u32 = suffix[1..].parse().expect("suffix is a number");
        assert!(num >= 2, "suffix number must be >= 2, got {num}");
    }
}
