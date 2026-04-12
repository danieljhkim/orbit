use std::fs;
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_types::{
    ActorIdentity, OrbitError, OrbitId, ReviewThread, Task, TaskArtifact, TaskComment,
    TaskComplexity, TaskHistoryEntry, TaskPriority, TaskStatus, TaskType,
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

    fn is_partitioned(self) -> bool {
        matches!(
            self,
            TaskStateDir::Done | TaskStateDir::Archived | TaskStateDir::Rejected
        )
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
    pr_status: Option<String>,
    #[serde(default)]
    source_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    batch_id: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    #[serde(default)]
    history: Vec<TaskHistoryEntry>,
    #[serde(default)]
    comments: Vec<TaskComment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    review_threads: Vec<ReviewThread>,
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
                acceptance_criteria: params.acceptance_criteria,
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
                pr_status: None,
                proposed_by: params.proposed_by,
                source_task_id: params.source_task_id,
                batch_id: None,
                created_at: now,
                updated_at: now,
                history: vec![TaskHistoryEntry {
                    at: now,
                    by: params.actor,
                    event: "created".to_string(),
                    note: None,
                    from_status: None,
                    to_status: Some(params.status),
                }],
                comments: params.comments,
                review_threads: Vec::new(),
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
            for task_dir in self.task_dirs_for_state(state)? {
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
        batch_id: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        let tasks = self.list_tasks()?;
        Ok(tasks
            .into_iter()
            .filter(|task| status.is_none_or(|value| task.status == value))
            .filter(|task| priority.is_none_or(|value| task.priority == value))
            .filter(|task| parent_id.is_none_or(|value| task.parent_id.as_deref() == Some(value)))
            .filter(|task| batch_id.is_none_or(|value| task.batch_id.as_deref() == Some(value)))
            .collect())
    }

    pub(crate) fn get_task(&self, id: &str) -> Result<Option<Task>, OrbitError> {
        let Some((state, task_dir)) = self.locate_task(id)? else {
            return Ok(None);
        };
        let bundle = self.read_bundle_at(&task_dir)?;
        Ok(Some(bundle_to_task(state, bundle)))
    }

    pub(crate) fn get_task_artifacts(
        &self,
        id: &str,
    ) -> Result<Option<Vec<TaskArtifact>>, OrbitError> {
        let Some((_, task_dir)) = self.locate_task(id)? else {
            return Ok(None);
        };
        Ok(Some(self.read_artifacts_at(&task_dir)?))
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
        if let Some(value) = &fields.acceptance_criteria {
            bundle.doc.acceptance_criteria = value.clone();
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
        if let Some(value) = &fields.pr_status {
            bundle.doc.pr_status = value.clone();
        }
        if let Some(value) = &fields.proposed_by {
            bundle.doc.proposed_by = value.clone();
        }
        if let Some(value) = &fields.source_task_id {
            bundle.doc.source_task_id = value.clone();
        }
        if let Some(value) = &fields.batch_id {
            bundle.doc.batch_id = value.clone();
        }
        if !fields.append_history.is_empty() {
            bundle.doc.history.extend(fields.append_history.clone());
        }
        if !fields.append_comments.is_empty() {
            bundle.doc.comments.extend(fields.append_comments.clone());
        }
        if let Some(ref threads) = fields.replace_review_threads {
            bundle.doc.review_threads = threads.clone();
        } else if !fields.append_review_threads.is_empty() {
            merge_review_threads(
                &mut bundle.doc.review_threads,
                fields.append_review_threads.clone(),
            );
        }
        let upsert_artifacts = fields.upsert_artifacts.clone();

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

        let final_dir = if target_state == current_state {
            self.write_bundle_at(&current_dir, &bundle)?;
            current_dir.clone()
        } else {
            self.write_bundle_at(&current_dir, &bundle)?;
            let target_dir = self.task_dir(target_state, &bundle.doc.id);
            self.move_task_dir(&current_dir, &target_dir)?;
            target_dir
        };
        if !upsert_artifacts.is_empty() {
            self.write_artifacts_at(&final_dir, &upsert_artifacts)?;
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
        let base = format!("T{}", now.format("%Y%m%d-%H%M"));
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
            if state.is_partitioned() {
                if let Some(partition) = partition_key(id) {
                    let partitioned_dir = self.state_dir_path(state).join(partition).join(id);
                    if partitioned_dir.is_dir() {
                        return Ok(Some((state, partitioned_dir)));
                    }
                }

                let legacy_dir = self.state_dir_path(state).join(id);
                if legacy_dir.is_dir() {
                    let migrated_dir = self.migrate_legacy_task_dir(state, legacy_dir)?;
                    return Ok(Some((state, migrated_dir)));
                }

                for partition_dir in self.partition_dirs(state)? {
                    let partitioned_dir = partition_dir.join(id);
                    if partitioned_dir.is_dir() {
                        return Ok(Some((state, partitioned_dir)));
                    }
                }
                continue;
            }

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
            plan: read_companion_text(&self.plan_path(task_dir), "task plan")?,
            execution_summary: read_companion_text(
                &self.execution_summary_path(task_dir),
                "task execution summary",
            )?,
        };
        self.validate_bundle(&bundle, Some(task_dir))?;
        Ok(bundle)
    }

    fn write_artifacts_at(
        &self,
        task_dir: &Path,
        artifacts: &[TaskArtifact],
    ) -> Result<(), OrbitError> {
        let artifacts_dir = self.artifacts_dir(task_dir);
        fs::create_dir_all(&artifacts_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
        for artifact in artifacts {
            let relative_path = normalize_artifact_path(&artifact.path)?;
            let destination = artifacts_dir.join(&relative_path);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;
            }
            write_atomic(&destination, &artifact.content)?;
        }
        Ok(())
    }

    fn read_artifacts_at(&self, task_dir: &Path) -> Result<Vec<TaskArtifact>, OrbitError> {
        let artifacts_dir = self.artifacts_dir(task_dir);
        if !artifacts_dir.exists() {
            return Ok(Vec::new());
        }

        let mut artifacts = Vec::new();
        collect_artifacts(&artifacts_dir, &artifacts_dir, &mut artifacts)?;
        artifacts.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(artifacts)
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
        if state.is_partitioned()
            && let Some(partition) = partition_key(id)
        {
            return self.state_dir_path(state).join(partition).join(id);
        }
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

    fn task_dirs_for_state(&self, state: TaskStateDir) -> Result<Vec<PathBuf>, OrbitError> {
        let state_dir = self.state_dir_path(state);
        if !state_dir.exists() {
            return Ok(Vec::new());
        }

        if !state.is_partitioned() {
            return read_child_dirs(&state_dir);
        }

        let mut task_dirs = Vec::new();
        for entry in read_child_dirs(&state_dir)? {
            let Some(name) = entry.file_name().and_then(|value| value.to_str()) else {
                continue;
            };

            if is_partition_dir_name(name) {
                task_dirs.extend(read_child_dirs(&entry)?);
                continue;
            }

            if self.task_doc_path(&entry).is_file() {
                task_dirs.push(self.migrate_legacy_task_dir(state, entry)?);
            }
        }

        Ok(task_dirs)
    }

    fn partition_dirs(&self, state: TaskStateDir) -> Result<Vec<PathBuf>, OrbitError> {
        let state_dir = self.state_dir_path(state);
        if !state_dir.exists() {
            return Ok(Vec::new());
        }

        Ok(read_child_dirs(&state_dir)?
            .into_iter()
            .filter(|path| {
                path.file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(is_partition_dir_name)
            })
            .collect())
    }

    fn migrate_legacy_task_dir(
        &self,
        state: TaskStateDir,
        legacy_dir: PathBuf,
    ) -> Result<PathBuf, OrbitError> {
        let Some(task_id) = legacy_dir.file_name().and_then(|value| value.to_str()) else {
            return Err(OrbitError::Store(format!(
                "invalid task directory path {}",
                legacy_dir.display()
            )));
        };
        let target_dir = self.task_dir(state, task_id);
        if target_dir == legacy_dir {
            return Ok(legacy_dir);
        }
        if target_dir.exists() {
            return Err(OrbitError::Store(format!(
                "cannot migrate task directory {} because {} already exists",
                legacy_dir.display(),
                target_dir.display()
            )));
        }
        self.move_task_dir(&legacy_dir, &target_dir)?;
        Ok(target_dir)
    }
}

fn partition_key(id: &str) -> Option<String> {
    let raw = id.strip_prefix('T')?;
    let year = raw.get(0..4)?;
    let month = raw.get(4..6)?;
    is_valid_year_month(year, month).then(|| format!("{year}-{month}"))
}

fn is_partition_dir_name(name: &str) -> bool {
    let Some((year, month)) = name.split_once('-') else {
        return false;
    };
    year.len() == 4 && month.len() == 2 && is_valid_year_month(year, month)
}

fn is_valid_year_month(year: &str, month: &str) -> bool {
    year.as_bytes().iter().all(u8::is_ascii_digit)
        && matches!(
            month,
            "01" | "02" | "03" | "04" | "05" | "06" | "07" | "08" | "09" | "10" | "11" | "12"
        )
}

fn read_child_dirs(dir: &Path) -> Result<Vec<PathBuf>, OrbitError> {
    let mut child_dirs = fs::read_dir(dir)
        .map_err(|e| OrbitError::Io(e.to_string()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    child_dirs.sort();
    Ok(child_dirs)
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
    yaml.push_str(&yaml_field("pr_status", &doc.pr_status)?);

    if doc.source_task_id.is_some() || doc.batch_id.is_some() {
        yaml.push_str(&yaml_section("attribution"));
        yaml.push_str(&yaml_field("source_task_id", &doc.source_task_id)?);
        if doc.batch_id.is_some() {
            yaml.push_str(&yaml_field("batch_id", &doc.batch_id)?);
        }
    }

    yaml.push_str(&yaml_section("timestamps"));
    yaml.push_str(&yaml_field("created_at", &doc.created_at)?);
    yaml.push_str(&yaml_field("updated_at", &doc.updated_at)?);

    yaml.push_str(&yaml_section("audit trail"));
    yaml.push_str(&yaml_field("history", &doc.history)?);
    yaml.push_str(&yaml_field("comments", &doc.comments)?);

    if !doc.review_threads.is_empty() {
        yaml.push_str(&yaml_section("review"));
        yaml.push_str(&yaml_field("review_threads", &doc.review_threads)?);
    }

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

fn read_companion_text(path: &Path, label: &str) -> Result<String, OrbitError> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(value),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(bundle_read_error(path, label, err)),
    }
}

fn bundle_read_error(path: &Path, label: &str, err: std::io::Error) -> OrbitError {
    if err.kind() == std::io::ErrorKind::NotFound {
        OrbitError::Store(format!("missing {label} at {}", path.display()))
    } else {
        OrbitError::Io(err.to_string())
    }
}

fn normalize_artifact_path(raw: &str) -> Result<String, OrbitError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(OrbitError::InvalidInput(
            "task artifact path must not be empty".to_string(),
        ));
    }

    let path = Path::new(trimmed);
    if path.is_absolute() {
        return Err(OrbitError::InvalidInput(format!(
            "task artifact path must be relative: {trimmed}"
        )));
    }

    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_str().ok_or_else(|| {
                    OrbitError::InvalidInput(format!(
                        "task artifact path must be valid UTF-8: {trimmed}"
                    ))
                })?;
                if part.is_empty() {
                    return Err(OrbitError::InvalidInput(format!(
                        "task artifact path must not contain empty segments: {trimmed}"
                    )));
                }
                parts.push(part.to_string());
            }
            _ => {
                return Err(OrbitError::InvalidInput(format!(
                    "task artifact path must stay within the task artifacts directory: {trimmed}"
                )));
            }
        }
    }

    if parts.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "task artifact path must contain a file name: {trimmed}"
        )));
    }

    Ok(parts.join("/"))
}

fn collect_artifacts(
    root: &Path,
    dir: &Path,
    artifacts: &mut Vec<TaskArtifact>,
) -> Result<(), OrbitError> {
    let mut entries = fs::read_dir(dir)
        .map_err(|e| OrbitError::Io(e.to_string()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    entries.sort();

    for entry in entries {
        if entry.is_dir() {
            collect_artifacts(root, &entry, artifacts)?;
            continue;
        }
        if !entry.is_file() {
            continue;
        }
        let relative = entry
            .strip_prefix(root)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let relative = normalize_artifact_path(&relative.to_string_lossy())?;
        let content = fs::read_to_string(&entry)
            .map_err(|e| bundle_read_error(&entry, "task artifact", e))?;
        artifacts.push(TaskArtifact {
            path: relative,
            content,
        });
    }

    Ok(())
}

/// Merge incoming review threads into existing threads.
///
/// If an incoming thread's `thread_id` matches an existing one, its messages
/// are appended and its status is updated. Otherwise the whole thread is added.
fn merge_review_threads(existing: &mut Vec<ReviewThread>, incoming: Vec<ReviewThread>) {
    for thread in incoming {
        if let Some(existing_thread) = existing
            .iter_mut()
            .find(|t| t.thread_id == thread.thread_id)
        {
            existing_thread.messages.extend(thread.messages);
            existing_thread.status = thread.status;
            if thread.github_thread_id.is_some() {
                existing_thread.github_thread_id = thread.github_thread_id;
            }
        } else {
            existing.push(thread);
        }
    }
}

fn bundle_to_task(state: TaskStateDir, bundle: TaskBundle) -> Task {
    // When loading from YAML, if actor_identity is System (the default) but
    // legacy agent/model fields are present, reconstruct the identity from them.
    let actor_identity = if bundle.doc.actor_identity.is_system() && bundle.doc.agent.is_some() {
        ActorIdentity::from_legacy(bundle.doc.agent.as_deref(), bundle.doc.model.as_deref())
    } else {
        bundle.doc.actor_identity
    };

    Task {
        id: bundle.doc.id,
        parent_id: bundle.doc.parent_id,
        title: bundle.doc.title,
        description: bundle.doc.description,
        acceptance_criteria: bundle.doc.acceptance_criteria,
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
        pr_status: bundle.doc.pr_status,
        proposed_by: bundle.doc.proposed_by,
        source_task_id: bundle.doc.source_task_id,
        batch_id: bundle.doc.batch_id,
        comments: bundle.doc.comments,
        history: bundle.doc.history,
        review_threads: bundle.doc.review_threads,
        created_at: bundle.doc.created_at,
        updated_at: bundle.doc.updated_at,
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn list_tasks_migrates_legacy_done_directories_into_month_partitions() {
        let temp_dir = tempdir().expect("create tempdir");
        let store = TaskFileStore::new(temp_dir.path().to_path_buf());
        store.ensure_layout().expect("create task layout");

        let task_id = "T20260315-123456";
        let legacy_dir = store.state_dir_path(TaskStateDir::Done).join(task_id);
        store
            .write_bundle_at(&legacy_dir, &sample_bundle(task_id, 15))
            .expect("write legacy task bundle");

        let tasks = store.list_tasks().expect("list tasks");

        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, task_id);
        assert_eq!(tasks[0].status, TaskStatus::Done);
        assert!(!legacy_dir.exists());
        assert!(
            store.task_dir(TaskStateDir::Done, task_id).exists(),
            "migrated task directory should exist under a yyyy-mm partition"
        );
    }

    #[test]
    fn get_task_migrates_legacy_archived_directories() {
        let temp_dir = tempdir().expect("create tempdir");
        let store = TaskFileStore::new(temp_dir.path().to_path_buf());
        store.ensure_layout().expect("create task layout");

        let task_id = "T20260401-091500";
        let legacy_dir = store.state_dir_path(TaskStateDir::Archived).join(task_id);
        store
            .write_bundle_at(&legacy_dir, &sample_bundle(task_id, 1))
            .expect("write legacy archived task bundle");

        let task = store
            .get_task(task_id)
            .expect("load task")
            .expect("task should exist");

        assert_eq!(task.id, task_id);
        assert_eq!(task.status, TaskStatus::Archived);
        assert!(!legacy_dir.exists());
        assert!(store.task_dir(TaskStateDir::Archived, task_id).exists());
    }

    #[test]
    fn task_dir_only_partitions_terminal_states() {
        let temp_dir = tempdir().expect("create tempdir");
        let store = TaskFileStore::new(temp_dir.path().to_path_buf());
        let task_id = "T20260331-005204";

        assert_eq!(
            store.task_dir(TaskStateDir::Done, task_id),
            temp_dir.path().join("done").join("2026-03").join(task_id)
        );
        assert_eq!(
            store.task_dir(TaskStateDir::Archived, task_id),
            temp_dir
                .path()
                .join("archived")
                .join("2026-03")
                .join(task_id)
        );
        assert_eq!(
            store.task_dir(TaskStateDir::Rejected, task_id),
            temp_dir
                .path()
                .join("rejected")
                .join("2026-03")
                .join(task_id)
        );
        assert_eq!(
            store.task_dir(TaskStateDir::Review, task_id),
            temp_dir.path().join("review").join(task_id)
        );
    }

    #[test]
    fn list_tasks_treats_missing_companion_files_as_blank() {
        let temp_dir = tempdir().expect("create tempdir");
        let store = TaskFileStore::new(temp_dir.path().to_path_buf());
        store.ensure_layout().expect("create task layout");

        let task_id = "T20260406-0454";
        let task_dir = store.task_dir(TaskStateDir::Proposed, task_id);
        store
            .write_bundle_at(&task_dir, &sample_bundle(task_id, 6))
            .expect("write task bundle");
        fs::remove_file(store.plan_path(&task_dir)).expect("remove plan companion");
        fs::remove_file(store.execution_summary_path(&task_dir))
            .expect("remove execution summary companion");

        let tasks = store.list_tasks().expect("list tasks");

        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, task_id);
        assert_eq!(tasks[0].plan, "");
        assert_eq!(tasks[0].execution_summary, "");
    }

    #[test]
    fn update_task_recreates_missing_companion_files() {
        let temp_dir = tempdir().expect("create tempdir");
        let store = TaskFileStore::new(temp_dir.path().to_path_buf());
        store.ensure_layout().expect("create task layout");

        let task_id = "T20260406-0454";
        let task_dir = store.task_dir(TaskStateDir::Proposed, task_id);
        store
            .write_bundle_at(&task_dir, &sample_bundle(task_id, 6))
            .expect("write task bundle");
        fs::remove_file(store.plan_path(&task_dir)).expect("remove plan companion");
        fs::remove_file(store.execution_summary_path(&task_dir))
            .expect("remove execution summary companion");

        let task = store
            .update_task(
                task_id,
                &TaskUpdateParams {
                    actor: "codex / gpt-5.4".to_string(),
                    plan: Some("repaired plan".to_string()),
                    execution_summary: Some("repaired summary".to_string()),
                    ..Default::default()
                },
            )
            .expect("update should recreate companion files");

        assert_eq!(task.plan, "repaired plan");
        assert_eq!(task.execution_summary, "repaired summary");
        assert_eq!(
            fs::read_to_string(store.plan_path(&task_dir)).expect("read plan companion"),
            "repaired plan"
        );
        assert_eq!(
            fs::read_to_string(store.execution_summary_path(&task_dir))
                .expect("read execution summary companion"),
            "repaired summary"
        );
    }

    #[test]
    fn task_artifacts_round_trip_through_update_and_load() {
        let temp_dir = tempdir().expect("create tempdir");
        let store = TaskFileStore::new(temp_dir.path().to_path_buf());
        store.ensure_layout().expect("create task layout");

        let task = store
            .create_task(TaskCreateParams {
                actor: "codex / gpt-5.4".to_string(),
                parent_id: None,
                title: "artifact task".to_string(),
                description: String::new(),
                acceptance_criteria: Vec::new(),
                plan: String::new(),
                execution_summary: String::new(),
                context_files: Vec::new(),
                workspace_path: None,
                repo_root: None,
                created_by: None,
                actor_identity: ActorIdentity::default(),
                assigned_to: None,
                status: TaskStatus::Backlog,
                priority: TaskPriority::Medium,
                complexity: None,
                task_type: TaskType::Task,
                pr_number: None,
                proposed_by: None,
                source_task_id: None,
                comments: Vec::new(),
            })
            .expect("create task");

        store
            .update_task(
                &task.id,
                &TaskUpdateParams {
                    actor: "codex / gpt-5.4".to_string(),
                    upsert_artifacts: vec![
                        TaskArtifact {
                            path: "planning-duel/codex-gpt-5.4.md".to_string(),
                            content: "*authored by: codex / gpt-5.4*\n## Plan\n- A".to_string(),
                        },
                        TaskArtifact {
                            path: "planning-duel/claude-opus.md".to_string(),
                            content: "*authored by: claude / opus*\n## Plan\n- B".to_string(),
                        },
                    ],
                    ..Default::default()
                },
            )
            .expect("write artifacts");

        let artifacts = store
            .get_task_artifacts(&task.id)
            .expect("load artifacts")
            .expect("task exists");

        assert_eq!(
            artifacts,
            vec![
                TaskArtifact {
                    path: "planning-duel/claude-opus.md".to_string(),
                    content: "*authored by: claude / opus*\n## Plan\n- B".to_string(),
                },
                TaskArtifact {
                    path: "planning-duel/codex-gpt-5.4.md".to_string(),
                    content: "*authored by: codex / gpt-5.4*\n## Plan\n- A".to_string(),
                },
            ]
        );
    }

    #[test]
    fn update_task_rejects_artifact_paths_outside_bundle() {
        let temp_dir = tempdir().expect("create tempdir");
        let store = TaskFileStore::new(temp_dir.path().to_path_buf());
        store.ensure_layout().expect("create task layout");

        let task = store
            .create_task(TaskCreateParams {
                actor: "codex / gpt-5.4".to_string(),
                parent_id: None,
                title: "artifact task".to_string(),
                description: String::new(),
                acceptance_criteria: Vec::new(),
                plan: String::new(),
                execution_summary: String::new(),
                context_files: Vec::new(),
                workspace_path: None,
                repo_root: None,
                created_by: None,
                actor_identity: ActorIdentity::default(),
                assigned_to: None,
                status: TaskStatus::Backlog,
                priority: TaskPriority::Medium,
                complexity: None,
                task_type: TaskType::Task,
                pr_number: None,
                proposed_by: None,
                source_task_id: None,
                comments: Vec::new(),
            })
            .expect("create task");

        let err = store
            .update_task(
                &task.id,
                &TaskUpdateParams {
                    actor: "codex / gpt-5.4".to_string(),
                    upsert_artifacts: vec![TaskArtifact {
                        path: "../escape.md".to_string(),
                        content: "bad".to_string(),
                    }],
                    ..Default::default()
                },
            )
            .expect_err("path traversal should fail");

        assert!(
            err.to_string()
                .contains("task artifact path must stay within the task artifacts directory")
        );
    }

    fn sample_bundle(task_id: &str, day: u32) -> TaskBundle {
        let timestamp = Utc
            .with_ymd_and_hms(2026, 3, day, 12, 34, 56)
            .single()
            .expect("valid timestamp");
        TaskBundle {
            doc: TaskFileDocument {
                schema_version: TASK_SCHEMA_VERSION,
                id: task_id.to_string(),
                parent_id: None,
                task_type: TaskType::Task,
                priority: TaskPriority::Medium,
                complexity: None,
                title: format!("Task {task_id}"),
                description: "Sample task".to_string(),
                acceptance_criteria: Vec::new(),
                context_files: Vec::new(),
                workspace_path: None,
                repo_root: None,
                created_by: None,
                actor_identity: ActorIdentity::default(),
                agent: None,
                model: None,
                assigned_to: None,
                proposed_by: None,
                pr_number: None,
                pr_status: None,
                source_task_id: None,
                batch_id: None,
                created_at: timestamp,
                updated_at: timestamp,
                history: Vec::new(),
                comments: Vec::new(),
                review_threads: Vec::new(),
            },
            plan: "plan".to_string(),
            execution_summary: String::new(),
        }
    }
}
