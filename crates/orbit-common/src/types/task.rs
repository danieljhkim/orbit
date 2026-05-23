//! Task types: status lifecycle, priority, complexity, and the [`Task`] struct itself.
//!
//! ## Task Status Lifecycle
//!
//! Transitions are **permissive by default** — any move is allowed unless it
//! violates one of the four invariants below.
//!
//! ### Invariants (blocklist)
//! 1. **Done is terminal** — no transitions out of done.
//! 2. **Archived requires dedicated command** — use `orbit task archive`; the
//!    bare `--status archived` path is rejected.
//! 3. **Friction is legacy-only** — new friction reports are stored through
//!    `orbit.friction.add`, not the task lifecycle.
//! 4. **InProgress → Review requires execution_summary** — enforced at the
//!    command layer, not in [`TaskStatus::validate_transition`].
//!
//! ### Statuses
//! | Status       | Purpose |
//! |--------------|---------|
//! | Proposed     | Awaiting human approval before entering the backlog. |
//! | Friction     | Legacy agent self-reported friction task. |
//! | Backlog      | Approved and queued for work. |
//! | Someday      | Future-scoped — wanted but not yet actionable. Agents skip someday tasks. |
//! | InProgress   | Actively being worked on. |
//! | Review       | Implementation complete; awaiting review/merge. |
//! | Done         | Accepted and closed. Terminal. |
//! | Blocked      | Temporarily paused. |
//! | Archived     | Soft-deleted. Restorable to Backlog. |
//! | Rejected     | Declined. Can be re-opened. |
//!
//! See [`TaskStatus::validate_transition`] for the blocklist implementation.

// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::str::FromStr;
use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::types::task_artifacts::{TaskRelation, TaskRelationType};
use crate::types::{OrbitError, OrbitId};
use crate::utility::selector::exists_in_workspace;

/// Current lifecycle state of a task.
///
/// See the module-level doc for the full state transition diagram.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Awaiting human approval before entering the backlog.
    Proposed,
    /// Legacy agent self-reported friction task.
    Friction,
    /// Approved and queued for work; not yet started.
    Backlog,
    /// Actively being worked on.
    #[cfg_attr(feature = "clap", value(name = "in-progress", alias = "in_progress"))]
    #[serde(alias = "in-progress")]
    InProgress,
    /// Implementation complete; awaiting review/merge.
    Review,
    /// Accepted and closed. Terminal — no further transitions.
    Done,
    /// Temporarily paused (waiting on a dependency or decision).
    Blocked,
    /// Soft-deleted. Can be restored to Backlog.
    Archived,
    /// Declined. Can be re-opened to Backlog or InProgress.
    Rejected,
    /// Future-scoped — wanted but not yet actionable. Agents skip someday tasks.
    Someday,
}

impl Display for TaskStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.cli_name())
    }
}

impl FromStr for TaskStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "proposed" => Ok(TaskStatus::Proposed),
            "friction" => Ok(TaskStatus::Friction),
            "backlog" => Ok(TaskStatus::Backlog),
            "in-progress" => Ok(TaskStatus::InProgress),
            "in_progress" => Ok(TaskStatus::InProgress),
            "review" => Ok(TaskStatus::Review),
            "done" => Ok(TaskStatus::Done),
            "blocked" => Ok(TaskStatus::Blocked),
            "archived" => Ok(TaskStatus::Archived),
            "rejected" => Ok(TaskStatus::Rejected),
            "someday" => Ok(TaskStatus::Someday),
            other => Err(format!("unknown task status: {other}")),
        }
    }
}

impl TaskStatus {
    pub fn cli_name(self) -> &'static str {
        match self {
            TaskStatus::Proposed => "proposed",
            TaskStatus::Friction => "friction",
            TaskStatus::Backlog => "backlog",
            TaskStatus::InProgress => "in-progress",
            TaskStatus::Review => "review",
            TaskStatus::Done => "done",
            TaskStatus::Blocked => "blocked",
            TaskStatus::Archived => "archived",
            TaskStatus::Rejected => "rejected",
            TaskStatus::Someday => "someday",
        }
    }

    /// Returns true when the status satisfies a task dependency.
    ///
    /// Orbit's lifecycle has no standalone terminal `approved` status for
    /// tasks today; accepted work lands in `done`.
    pub fn satisfies_dependency(self) -> bool {
        matches!(self, TaskStatus::Done)
    }

    /// Validates a status transition using a short blocklist of invariants:
    ///
    /// 1. **Done is terminal** — no transitions out of done.
    /// 2. **Archived requires dedicated command** — use `orbit task archive`, not a
    ///    bare status update (enforced upstream; blocked here as defense-in-depth).
    /// 3. **Friction is legacy-only** — new friction reports use
    ///    `orbit.friction.add`.
    /// 4. **InProgress → Review requires execution_summary** — enforced upstream in
    ///    `update_task_with_status_note`, not here (we lack the task data).
    ///
    /// Everything else is allowed.
    pub fn validate_transition(&self, target: TaskStatus) -> Result<(), String> {
        // No-op transitions are always fine.
        if *self == target {
            return Ok(());
        }

        // Done is terminal.
        if *self == TaskStatus::Done {
            return Err(format!(
                "invalid status transition: {} -> {} (done is terminal)",
                self, target
            ));
        }

        // Archived requires the dedicated archive command.
        if target == TaskStatus::Archived {
            return Err(format!(
                "invalid status transition: {} -> {} (use the archive command)",
                self, target
            ));
        }

        // Friction is retained for legacy persisted tasks only. New friction
        // reports are append-only records under `.orbit/frictions/`.
        if target == TaskStatus::Friction {
            return Err(format!(
                "invalid status transition: {} -> {} (friction can only be set at task creation)",
                self, target
            ));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    Low,
    Medium,
    High,
    Critical,
}

impl Display for TaskPriority {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            TaskPriority::Low => "low",
            TaskPriority::Medium => "medium",
            TaskPriority::High => "high",
            TaskPriority::Critical => "critical",
        };
        write!(f, "{s}")
    }
}

impl FromStr for TaskPriority {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "low" => Ok(TaskPriority::Low),
            "medium" => Ok(TaskPriority::Medium),
            "high" => Ok(TaskPriority::High),
            "critical" => Ok(TaskPriority::Critical),
            other => Err(format!("unknown task priority: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum TaskComplexity {
    Low,
    Medium,
    Hard,
}

impl Display for TaskComplexity {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            TaskComplexity::Low => "low",
            TaskComplexity::Medium => "medium",
            TaskComplexity::Hard => "hard",
        };
        write!(f, "{s}")
    }
}

impl FromStr for TaskComplexity {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "low" => Ok(TaskComplexity::Low),
            "medium" => Ok(TaskComplexity::Medium),
            "hard" => Ok(TaskComplexity::Hard),
            other => Err(format!("unknown task complexity: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    Feature,
    /// An attributable defect; lineage is recorded through typed task relations.
    Bug,
    Refactor,
    Chore,
}

impl Display for TaskType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            TaskType::Feature => "feature",
            TaskType::Bug => "bug",
            TaskType::Refactor => "refactor",
            TaskType::Chore => "chore",
        };
        write!(f, "{s}")
    }
}

impl FromStr for TaskType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "feature" => Ok(TaskType::Feature),
            "bug" => Ok(TaskType::Bug),
            "refactor" => Ok(TaskType::Refactor),
            "chore" => Ok(TaskType::Chore),
            other => Err(format!(
                "unknown task type: {other} (valid types: {})",
                TaskType::valid_names().join(", ")
            )),
        }
    }
}

impl TaskType {
    pub fn valid_names() -> &'static [&'static str] {
        &["feature", "bug", "refactor", "chore"]
    }
}

/// Status of a review thread (open or resolved).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewThreadStatus {
    Open,
    Resolved,
}

impl Display for ReviewThreadStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ReviewThreadStatus::Open => write!(f, "open"),
            ReviewThreadStatus::Resolved => write!(f, "resolved"),
        }
    }
}

impl FromStr for ReviewThreadStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "open" => Ok(ReviewThreadStatus::Open),
            "resolved" => Ok(ReviewThreadStatus::Resolved),
            other => Err(format!("unknown review thread status: {other}")),
        }
    }
}

/// A single message within a [`ReviewThread`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewMessage {
    pub message_id: String,
    pub at: DateTime<Utc>,
    pub by: String,
    pub body: String,
    /// GitHub comment ID, set after sync. `None` means pending sync.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_comment_id: Option<u64>,
}

/// Anchor kind for a [`ReviewThread`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewThreadAnchor {
    Inline { path: String, line: u64 },
    TaskLevel,
}

/// A review thread on a task, replacing direct GitHub review comments.
///
/// Threads with `path` and `line` are inline (file-specific) comments.
/// Threads without are general comments (e.g. review summaries).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ReviewThread {
    pub thread_id: String,
    /// File path relative to repo root. `None` for general comments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Line number in the file. `None` for general comments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    pub status: ReviewThreadStatus,
    pub messages: Vec<ReviewMessage>,
    /// GitHub review thread/comment ID, set after first sync.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_thread_id: Option<u64>,
}

impl ReviewThread {
    pub fn anchor(&self) -> ReviewThreadAnchor {
        match (&self.path, self.line) {
            (Some(path), Some(line)) => ReviewThreadAnchor::Inline {
                path: path.clone(),
                line,
            },
            _ => ReviewThreadAnchor::TaskLevel,
        }
    }
}

impl Serialize for ReviewThread {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("ReviewThread", 7)?;
        state.serialize_field("thread_id", &self.thread_id)?;
        state.serialize_field("anchor", &self.anchor())?;
        if let Some(path) = &self.path {
            state.serialize_field("path", path)?;
        }
        if let Some(line) = self.line {
            state.serialize_field("line", &line)?;
        }
        state.serialize_field("status", &self.status)?;
        state.serialize_field("messages", &self.messages)?;
        if let Some(github_thread_id) = self.github_thread_id {
            state.serialize_field("github_thread_id", &github_thread_id)?;
        }
        state.end()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskComment {
    pub at: DateTime<Utc>,
    pub by: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskHistoryEntry {
    pub at: DateTime<Utc>,
    pub by: String,
    pub event: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_status: Option<TaskStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_status: Option<TaskStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskArtifact {
    pub path: String,
    #[serde(default)]
    pub content: Vec<u8>,
    #[serde(default = "default_task_artifact_media_type")]
    pub media_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
}

impl TaskArtifact {
    pub fn from_text(path: impl Into<String>, content: impl Into<String>) -> Self {
        let path = path.into();
        let content = content.into();
        Self {
            media_type: media_type_for_artifact_path(&path).to_string(),
            path,
            content: content.into_bytes(),
            created_by: None,
        }
    }

    pub fn text_content(&self) -> Option<&str> {
        std::str::from_utf8(&self.content).ok()
    }

    pub fn from_source_file(
        source_path: &Path,
        artifact_path: Option<&str>,
    ) -> Result<Self, OrbitError> {
        let metadata = std::fs::metadata(source_path).map_err(|error| {
            OrbitError::InvalidInput(format!(
                "cannot read task artifact source '{}': {error}",
                source_path.display()
            ))
        })?;
        if !metadata.is_file() {
            return Err(OrbitError::InvalidInput(format!(
                "task artifact source '{}' must be a file",
                source_path.display()
            )));
        }

        let path = match artifact_path {
            Some(path) => {
                let trimmed = path.trim();
                if trimmed.is_empty() {
                    return Err(OrbitError::InvalidInput(
                        "task artifact path must not be empty".to_string(),
                    ));
                }
                trimmed.to_string()
            }
            None => infer_artifact_path_from_source(source_path)?,
        };
        let content = std::fs::read(source_path).map_err(|error| {
            OrbitError::InvalidInput(format!(
                "cannot read task artifact source '{}': {error}",
                source_path.display()
            ))
        })?;

        Ok(Self {
            media_type: media_type_for_artifact_path(&path).to_string(),
            path,
            content,
            created_by: None,
        })
    }
}

fn default_task_artifact_media_type() -> String {
    "application/octet-stream".to_string()
}

pub fn media_type_for_artifact_path(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("md" | "markdown") => "text/markdown",
        Some("txt" | "log") => "text/plain",
        Some("json") => "application/json",
        Some("yaml" | "yml") => "application/yaml",
        Some("toml") => "application/toml",
        Some("html" | "htm") => "text/html",
        Some("csv") => "text/csv",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn infer_artifact_path_from_source(source_path: &Path) -> Result<String, OrbitError> {
    let file_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "cannot infer task artifact path from source '{}'; pass --path",
                source_path.display()
            ))
        })?;
    Ok(file_name.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedTaskDependency {
    pub id: OrbitId,
    pub status: String,
}

impl ResolvedTaskDependency {
    pub fn label(&self) -> String {
        format!("{} [{}]", self.id, self.status)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ExternalRef {
    pub system: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

pub const GITHUB_PR_EXTERNAL_REF_SYSTEM: &str = "github-pr";

impl ExternalRef {
    pub fn try_new(system: String, id: String, url: Option<String>) -> Result<Self, OrbitError> {
        let system = Self::validate_system(&system)?;

        let id = id.trim();
        if id.is_empty() {
            return Err(OrbitError::InvalidInput(
                "external ref id must not be empty".to_string(),
            ));
        }

        let url = url
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| {
                Url::parse(&value).map_err(|error| {
                    OrbitError::InvalidInput(format!(
                        "external ref url '{value}' must be a valid URL: {error}"
                    ))
                })?;
                Ok::<String, OrbitError>(value)
            })
            .transpose()?;

        Ok(Self {
            system,
            id: id.to_string(),
            url,
        })
    }

    pub fn is_valid_system(system: &str) -> bool {
        external_ref_system_regex().is_match(system.trim())
    }

    pub fn validate_system(system: &str) -> Result<String, OrbitError> {
        let system = system.trim();
        if !Self::is_valid_system(system) {
            return Err(OrbitError::InvalidInput(format!(
                "external ref system '{system}' must match ^[a-z][a-z0-9-]*$"
            )));
        }
        Ok(system.to_string())
    }

    pub fn parse_key(raw: &str) -> Result<Self, OrbitError> {
        let (system, id) = raw.split_once(':').ok_or_else(|| {
            OrbitError::InvalidInput(
                "external ref must use <system>:<id> form, for example jira:ENG-1234".to_string(),
            )
        })?;
        Self::try_new(system.to_string(), id.to_string(), None)
    }

    pub fn github_pr(id: impl Into<String>) -> Result<Self, OrbitError> {
        Self::try_new(GITHUB_PR_EXTERNAL_REF_SYSTEM.to_string(), id.into(), None)
    }

    pub fn has_key(&self, system: &str, id: &str) -> bool {
        self.system == system && self.id == id
    }
}

pub fn push_external_ref_if_missing(refs: &mut Vec<ExternalRef>, external_ref: ExternalRef) {
    if !refs
        .iter()
        .any(|candidate| candidate.has_key(&external_ref.system, &external_ref.id))
    {
        refs.push(external_ref);
    }
}

impl<'de> Deserialize<'de> for ExternalRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawExternalRef {
            system: String,
            id: String,
            #[serde(default)]
            url: Option<String>,
        }

        let raw = RawExternalRef::deserialize(deserializer)?;
        ExternalRef::try_new(raw.system, raw.id, raw.url).map_err(serde::de::Error::custom)
    }
}

fn external_ref_system_regex() -> &'static Regex {
    static SYSTEM_REGEX: OnceLock<Regex> = OnceLock::new();
    SYSTEM_REGEX.get_or_init(|| {
        Regex::new(r"^[a-z][a-z0-9-]*$").expect("external ref system regex is valid")
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Task {
    pub id: OrbitId,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, alias = "instructions")]
    pub plan: String,
    #[serde(default)]
    pub execution_summary: String,
    pub context_files: Vec<String>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub planned_by: Option<String>,
    #[serde(default)]
    pub implemented_by: Option<String>,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    #[serde(default)]
    pub complexity: Option<TaskComplexity>,
    pub task_type: TaskType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_status: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_refs: Vec<ExternalRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relations: Vec<TaskRelation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crew: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Display for Task {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}\t{}\t{}\t{}",
            self.id, self.status, self.priority, self.title
        )
    }
}

impl Task {
    pub fn parsed_plan(&self) -> Result<crate::types::task_plan::TaskPlan, OrbitError> {
        let label = format!("task '{}' plan", self.id);
        crate::types::task_plan::parse_task_plan(self.plan.as_str(), label.as_str())
    }

    pub fn github_pr_number(&self) -> Option<&str> {
        self.external_refs
            .iter()
            .find(|external_ref| external_ref.system == GITHUB_PR_EXTERNAL_REF_SYSTEM)
            .map(|external_ref| external_ref.id.as_str())
    }

    pub fn parent_id(&self) -> Option<&str> {
        self.relation_target(TaskRelationType::ChildOf)
    }

    pub fn dependencies(&self) -> Vec<OrbitId> {
        self.relation_targets(TaskRelationType::BlockedBy)
    }

    pub fn source_task_id(&self) -> Option<&str> {
        self.relation_target(TaskRelationType::RegressionFrom)
    }

    fn relation_target(&self, relation_type: TaskRelationType) -> Option<&str> {
        self.relations
            .iter()
            .find(|relation| relation.relation_type == relation_type)
            .map(|relation| relation.target.as_str())
    }

    fn relation_targets(&self, relation_type: TaskRelationType) -> Vec<OrbitId> {
        self.relations
            .iter()
            .filter(|relation| relation.relation_type == relation_type)
            .map(|relation| relation.target.clone())
            .collect()
    }
}

/// Partition candidate `context_files` into `(kept, dropped)` based on filesystem existence.
///
/// - Empty / whitespace-only entries are silently discarded (not reported as dropped).
/// - Each remaining entry is trimmed of leading/trailing whitespace before use.
/// - Selector anchors and relative paths are resolved against `workspace_root`.
/// - Absolute paths are checked as-is.
///
/// Entries whose resolved path does not exist on disk end up in `dropped` (as the
/// trimmed, but otherwise unmodified, original string). Entries that do exist end
/// up in `kept` (also trimmed).
pub fn prune_missing_context_files(
    workspace_root: &std::path::Path,
    candidates: Vec<String>,
) -> (Vec<String>, Vec<String>) {
    let mut kept = Vec::with_capacity(candidates.len());
    let mut dropped = Vec::new();
    for entry in candidates {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        if exists_in_workspace(trimmed, workspace_root) {
            kept.push(trimmed.to_string());
        } else {
            dropped.push(trimmed.to_string());
        }
    }
    (kept, dropped)
}

pub fn normalize_task_dependencies(
    raw_dependencies: Vec<String>,
) -> Result<Vec<OrbitId>, OrbitError> {
    let mut normalized = Vec::with_capacity(raw_dependencies.len());
    let mut seen = BTreeSet::new();
    for raw in raw_dependencies {
        let dependency = raw.trim();
        if dependency.is_empty() {
            return Err(OrbitError::InvalidInput(
                "task dependencies must not contain empty IDs".to_string(),
            ));
        }
        if seen.insert(dependency.to_string()) {
            normalized.push(dependency.to_string());
        }
    }
    Ok(normalized)
}

pub fn normalize_task_tags(raw_tags: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::with_capacity(raw_tags.len());
    let mut seen = BTreeSet::new();
    for raw in raw_tags {
        let tag = raw.trim().to_lowercase();
        if !tag.is_empty() && seen.insert(tag.clone()) {
            normalized.push(tag);
        }
    }
    normalized
}

pub fn task_matches_tags(task: &Task, required_tags: &[String]) -> bool {
    let required_tags = normalize_task_tags(required_tags.to_vec());
    if required_tags.is_empty() {
        return true;
    }

    let available = normalize_task_tags(task.tags.clone())
        .into_iter()
        .collect::<BTreeSet<_>>();
    required_tags
        .iter()
        .all(|tag| available.contains(tag.as_str()))
}

pub fn build_task_status_index(tasks: &[Task]) -> BTreeMap<OrbitId, TaskStatus> {
    tasks
        .iter()
        .map(|task| (task.id.clone(), task.status))
        .collect::<BTreeMap<_, _>>()
}

pub fn resolve_task_dependencies(
    task: &Task,
    status_by_id: &BTreeMap<OrbitId, TaskStatus>,
) -> Vec<ResolvedTaskDependency> {
    task.dependencies()
        .into_iter()
        .map(|dependency_id| ResolvedTaskDependency {
            id: dependency_id.clone(),
            status: status_by_id
                .get(&dependency_id)
                .map(|status| status.to_string())
                .unwrap_or_else(|| "missing".to_string()),
        })
        .collect()
}

pub fn task_dependencies_ready(task: &Task, status_by_id: &BTreeMap<OrbitId, TaskStatus>) -> bool {
    task.dependencies().iter().all(|dependency_id| {
        status_by_id
            .get(dependency_id)
            .is_some_and(|status| status.satisfies_dependency())
    })
}

pub fn unmet_task_dependencies(
    task: &Task,
    status_by_id: &BTreeMap<OrbitId, TaskStatus>,
) -> Vec<ResolvedTaskDependency> {
    resolve_task_dependencies(task, status_by_id)
        .into_iter()
        .filter(|dependency| {
            status_by_id
                .get(&dependency.id)
                .is_none_or(|status| !status.satisfies_dependency())
        })
        .collect()
}

pub fn validate_task_dependencies(
    tasks: &[Task],
    current_task_id: Option<&str>,
    dependencies: &[OrbitId],
) -> Result<(), OrbitError> {
    let task_ids = tasks
        .iter()
        .map(|task| task.id.clone())
        .collect::<BTreeSet<_>>();
    for dependency in dependencies {
        if !task_ids.contains(dependency) {
            return Err(OrbitError::InvalidInput(format!(
                "task dependency '{dependency}' does not resolve in this workspace"
            )));
        }
    }

    let Some(current_task_id) = current_task_id else {
        return Ok(());
    };

    if dependencies
        .iter()
        .any(|dependency| dependency == current_task_id)
    {
        return Err(OrbitError::InvalidInput(format!(
            "task '{current_task_id}' cannot declare a self-dependency (self-reference)"
        )));
    }

    let mut adjacency = tasks
        .iter()
        .map(|task| (task.id.clone(), task.dependencies()))
        .collect::<BTreeMap<_, _>>();
    adjacency.insert(current_task_id.to_string(), dependencies.to_vec());

    for dependency in dependencies {
        let mut visiting = BTreeSet::new();
        let mut trail = Vec::new();
        if let Some(path) = find_dependency_path(
            dependency,
            current_task_id,
            &adjacency,
            &mut visiting,
            &mut trail,
        ) {
            let mut cycle = Vec::with_capacity(path.len() + 1);
            cycle.push(current_task_id.to_string());
            cycle.extend(path);
            return Err(OrbitError::InvalidInput(format!(
                "task dependency cycle detected: {}",
                cycle.join(" -> ")
            )));
        }
    }

    Ok(())
}

fn find_dependency_path(
    current: &str,
    target: &str,
    adjacency: &BTreeMap<OrbitId, Vec<OrbitId>>,
    visiting: &mut BTreeSet<OrbitId>,
    trail: &mut Vec<OrbitId>,
) -> Option<Vec<OrbitId>> {
    if !visiting.insert(current.to_string()) {
        return None;
    }

    trail.push(current.to_string());
    if current == target {
        return Some(trail.clone());
    }

    if let Some(next_dependencies) = adjacency.get(current) {
        for next in next_dependencies {
            if let Some(path) = find_dependency_path(next, target, adjacency, visiting, trail) {
                return Some(path);
            }
        }
    }

    trail.pop();
    visiting.remove(current);
    None
}
