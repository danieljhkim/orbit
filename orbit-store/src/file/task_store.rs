use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_types::{OrbitError, Task, TaskPriority, TaskStatus, TaskType};
use serde::{Deserialize, Serialize};

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
    schema_version: u8,
    id: String,
    title: String,
    description: String,
    #[serde(default)]
    instructions: String,
    #[serde(default)]
    #[serde(rename = "execution_summary")]
    #[serde(alias = "executionSummary")]
    execution_summary: String,
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
        let doc = TaskFileDocument {
            schema_version: 1,
            id,
            title: params.title,
            description: params.description,
            instructions: params.instructions,
            execution_summary: params.execution_summary,
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
        };

        self.write_doc_for_state(initial_state, &doc)?;
        Ok(doc_to_task(initial_state, doc))
    }

    pub(crate) fn list_tasks(&self) -> Result<Vec<Task>, OrbitError> {
        let mut tasks = Vec::new();
        for state in TaskStateDir::all() {
            let dir = self.state_dir_path(state);
            if !dir.exists() {
                continue;
            }
            let mut paths = fs::read_dir(&dir)
                .map_err(|e| OrbitError::Io(e.to_string()))?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| is_yaml(path))
                .collect::<Vec<_>>();
            paths.sort();

            for path in paths {
                let doc = self.read_doc_at(&path)?;
                tasks.push(doc_to_task(state, doc));
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
        let Some((state, path)) = self.locate_task(id)? else {
            return Ok(None);
        };
        let doc = self.read_doc_at(&path)?;
        Ok(Some(doc_to_task(state, doc)))
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
        let Some((current_state, current_path)) = self.locate_task(id)? else {
            return Err(OrbitError::TaskNotFound(id.to_string()));
        };
        let mut doc = self.read_doc_at(&current_path)?;

        if let Some(value) = &fields.title {
            doc.title = value.clone();
        }
        if let Some(value) = &fields.description {
            doc.description = value.clone();
        }
        if let Some(value) = &fields.instructions {
            doc.instructions = value.clone();
        }
        if let Some(value) = &fields.execution_summary {
            doc.execution_summary = value.clone();
        }
        if let Some(value) = &fields.context_files {
            doc.context_files = value.clone();
        }
        if let Some(value) = &fields.workspace_path {
            doc.workspace_path = value.clone();
        }
        if let Some(value) = &fields.assigned_to {
            doc.assigned_to = value.clone();
        }
        if let Some(value) = &fields.created_by {
            doc.created_by = value.clone();
        }
        if let Some(value) = fields.priority {
            doc.priority = value;
        }
        if let Some(value) = fields.task_type {
            doc.task_type = value;
        }
        if let Some(value) = &fields.branch {
            doc.branch = value.clone();
        }
        if let Some(value) = &fields.pr_number {
            doc.pr_number = value.clone();
        }
        if let Some(value) = &fields.proposed_by {
            doc.proposed_by = value.clone();
        }
        if let Some(value) = &fields.proposal_approved_by {
            doc.proposal_approved_by = value.clone();
        }
        if let Some(value) = &fields.proposal_decision_note {
            doc.proposal_decision_note = value.clone();
        }
        if let Some(value) = &fields.review_approved_by {
            doc.review_approved_by = value.clone();
        }
        if let Some(value) = &fields.review_decision_note {
            doc.review_decision_note = value.clone();
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

        doc.updated_at = Utc::now();
        if let Some(event) = event {
            doc.history.push(TaskHistoryEntry {
                at: doc.updated_at,
                by: "human".to_string(),
                event,
            });
        }

        self.validate_doc(&doc)?;
        let target_path = self.task_path(target_state, &doc.id);
        self.write_doc_at(&target_path, &doc)?;
        if target_path != current_path {
            fs::remove_file(&current_path).map_err(|e| OrbitError::Io(e.to_string()))?;
        }

        Ok(doc_to_task(target_state, doc))
    }

    pub(crate) fn delete_task(&self, id: &str) -> Result<bool, OrbitError> {
        let Some((_, path)) = self.locate_task(id)? else {
            return Ok(false);
        };
        fs::remove_file(path).map_err(|e| OrbitError::Io(e.to_string()))?;
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
            let path = self.task_path(state, id);
            if path.exists() {
                return Ok(Some((state, path)));
            }
        }
        Ok(None)
    }

    fn write_doc_for_state(
        &self,
        state: TaskStateDir,
        doc: &TaskFileDocument,
    ) -> Result<(), OrbitError> {
        self.validate_doc(doc)?;
        let path = self.task_path(state, &doc.id);
        self.write_doc_at(&path, doc)
    }

    fn write_doc_at(&self, path: &Path, doc: &TaskFileDocument) -> Result<(), OrbitError> {
        self.validate_doc(doc)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;
        }

        let yaml = serialize_task_doc_yaml(doc)?;
        let nanos = Utc::now().timestamp_nanos_opt().unwrap_or_default();
        let tmp_path = path.with_extension(format!("yaml.tmp.{nanos}"));
        fs::write(&tmp_path, yaml).map_err(|e| OrbitError::Io(e.to_string()))?;
        if let Err(err) = fs::rename(&tmp_path, path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(OrbitError::Io(err.to_string()));
        }
        Ok(())
    }

    fn read_doc_at(&self, path: &Path) -> Result<TaskFileDocument, OrbitError> {
        let raw = fs::read_to_string(path).map_err(|e| OrbitError::Io(e.to_string()))?;
        let doc = serde_yaml::from_str::<TaskFileDocument>(&raw)
            .map_err(|e| OrbitError::Store(format!("invalid task file {}: {e}", path.display())))?;
        self.validate_doc(&doc)?;
        Ok(doc)
    }

    fn validate_doc(&self, doc: &TaskFileDocument) -> Result<(), OrbitError> {
        if doc.schema_version != 1 {
            return Err(OrbitError::InvalidInput(format!(
                "unsupported task schema version: {}",
                doc.schema_version
            )));
        }
        if doc.id.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task id must not be empty".to_string(),
            ));
        }
        if doc.title.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task title must not be empty".to_string(),
            ));
        }
        Ok(())
    }

    fn state_dir_path(&self, state: TaskStateDir) -> PathBuf {
        self.root.join(state.as_dir())
    }

    fn task_path(&self, state: TaskStateDir, id: &str) -> PathBuf {
        self.state_dir_path(state).join(format!("{id}.yaml"))
    }
}

fn is_yaml(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("yaml") | Some("yml")
    )
}

fn serialize_task_doc_yaml(doc: &TaskFileDocument) -> Result<String, OrbitError> {
    let yaml = serde_yaml::to_string(doc).map_err(|e| OrbitError::Store(e.to_string()))?;
    let yaml =
        rewrite_top_level_string_field_as_literal_block(&yaml, "description", &doc.description);
    let yaml =
        rewrite_top_level_string_field_as_literal_block(&yaml, "instructions", &doc.instructions);
    let yaml = rewrite_top_level_string_field_as_literal_block(
        &yaml,
        "execution_summary",
        &doc.execution_summary,
    );
    Ok(yaml)
}

fn rewrite_top_level_string_field_as_literal_block(yaml: &str, key: &str, value: &str) -> String {
    let lines: Vec<&str> = yaml.lines().collect();
    let Some(start_idx) = lines
        .iter()
        .position(|line| line.starts_with(&format!("{key}:")))
    else {
        return yaml.to_string();
    };

    let mut end_idx = start_idx + 1;
    while end_idx < lines.len() {
        let line = lines[end_idx];
        if !line.starts_with(' ') && line.contains(':') {
            break;
        }
        end_idx += 1;
    }

    let mut output: Vec<String> = lines[..start_idx]
        .iter()
        .map(|line| (*line).to_string())
        .collect();

    if value.is_empty() {
        output.push(format!("{key}: |-"));
    } else {
        output.push(format!("{key}: |"));
        for part in value.split('\n') {
            output.push(format!("  {part}"));
        }
    }

    output.extend(lines[end_idx..].iter().map(|line| (*line).to_string()));
    let mut rendered = output.join("\n");
    rendered.push('\n');
    rendered
}

fn doc_to_task(state: TaskStateDir, doc: TaskFileDocument) -> Task {
    Task {
        id: doc.id,
        title: doc.title,
        description: doc.description,
        instructions: doc.instructions,
        execution_summary: doc.execution_summary,
        context_files: doc.context_files,
        workspace_path: doc.workspace_path,
        assigned_to: doc.assigned_to,
        created_by: doc.created_by,
        status: state.to_status(),
        priority: doc.priority,
        task_type: doc.task_type,
        branch: doc.branch,
        pr_number: doc.pr_number,
        proposed_by: doc.proposed_by,
        proposal_approved_by: doc.proposal_approved_by,
        proposal_decision_note: doc.proposal_decision_note,
        review_approved_by: doc.review_approved_by,
        review_decision_note: doc.review_decision_note,
        created_at: doc.created_at,
        updated_at: doc.updated_at,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TaskFileDocument, rewrite_top_level_string_field_as_literal_block, serialize_task_doc_yaml,
    };
    use orbit_types::TaskPriority;

    #[test]
    fn rewrite_emits_literal_block_for_multiline() {
        let yaml = "id: T1\ndescription: one line\ninstructions: short\nexecution_summary: \"\"\npriority: medium\n";
        let updated =
            rewrite_top_level_string_field_as_literal_block(yaml, "description", "Line 1\nLine 2");
        assert!(updated.contains("description: |\n  Line 1\n  Line 2\n"));
    }

    #[test]
    fn serialize_task_doc_uses_literal_blocks_for_target_fields() {
        let now = chrono::Utc::now();
        let doc = TaskFileDocument {
            schema_version: 1,
            id: "T1".to_string(),
            title: "Title".to_string(),
            description: "Desc line 1\nDesc line 2".to_string(),
            instructions: "Step 1\nStep 2".to_string(),
            execution_summary: "Implemented and tested".to_string(),
            context_files: vec![],
            workspace_path: None,
            assigned_to: None,
            created_by: None,
            priority: TaskPriority::Medium,
            task_type: super::default_task_type(),
            branch: None,
            pr_number: None,
            proposed_by: None,
            proposal_approved_by: None,
            proposal_decision_note: None,
            review_approved_by: None,
            review_decision_note: None,
            created_at: now,
            updated_at: now,
            acceptance_criteria: vec![],
            history: vec![],
            comments: vec![],
            job_id: None,
            scheduler_id: None,
            scheduler_run_id: None,
        };

        let yaml = serialize_task_doc_yaml(&doc).expect("yaml");
        assert!(yaml.contains("description: |\n  Desc line 1\n  Desc line 2\n"));
        assert!(yaml.contains("instructions: |\n  Step 1\n  Step 2\n"));
        assert!(yaml.contains("execution_summary: |\n  Implemented and tested\n"));
    }
}
