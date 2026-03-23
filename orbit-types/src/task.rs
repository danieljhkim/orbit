//! Task types: status lifecycle, priority, complexity, and the [`Task`] struct itself.
//!
//! ## Task Status Lifecycle
//!
//! Transitions are **permissive by default** — any move is allowed unless it
//! violates one of the three invariants below.
//!
//! ### Invariants (blocklist)
//! 1. **Done is terminal** — no transitions out of done.
//! 2. **Archived requires dedicated command** — use `orbit task archive`; the
//!    bare `--status archived` path is rejected.
//! 3. **InProgress → Review requires execution_summary** — enforced at the
//!    command layer, not in [`TaskStatus::validate_transition`].
//!
//! ### Statuses
//! | Status       | Purpose |
//! |--------------|---------|
//! | Proposed     | Awaiting human approval before entering the backlog. |
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

use std::fmt::{Display, Formatter};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::OrbitId;

/// Current lifecycle state of a task.
///
/// See the module-level doc for the full state transition diagram.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Awaiting human approval before entering the backlog.
    Proposed,
    /// Approved and queued for work; not yet started.
    Backlog,
    /// Actively being worked on.
    #[cfg_attr(feature = "clap", value(name = "in-progress", alias = "in_progress"))]
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

    /// Validates a status transition using a short blocklist of invariants:
    ///
    /// 1. **Done is terminal** — no transitions out of done.
    /// 2. **Archived requires dedicated command** — use `orbit task archive`, not a
    ///    bare status update (enforced upstream; blocked here as defense-in-depth).
    /// 3. **InProgress → Review requires execution_summary** — enforced upstream in
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
    Task,
    Feature,
    /// Agent-reported friction, DX issues, or system problems.
    /// Preferred type for agent issue reports — triggers scoreboard hooks.
    Friction,
    /// Legacy alias for Friction. Both types trigger scoreboard hooks.
    #[serde(alias = "issue")]
    Issue,
    /// An attributable defect — tracks which agent/model introduced the bug
    /// via the `agent`, `model`, and `source_task_id` fields on [`Task`].
    Bug,
    #[cfg_attr(feature = "clap", value(alias = "other"))]
    #[serde(alias = "other")]
    Chore,
    Refactor,
}

impl Display for TaskType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            TaskType::Task => "task",
            TaskType::Feature => "feature",
            TaskType::Friction => "friction",
            TaskType::Issue => "issue",
            TaskType::Bug => "bug",
            TaskType::Chore => "chore",
            TaskType::Refactor => "refactor",
        };
        write!(f, "{s}")
    }
}

impl FromStr for TaskType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "task" => Ok(TaskType::Task),
            "feature" => Ok(TaskType::Feature),
            "friction" => Ok(TaskType::Friction),
            "issue" => Ok(TaskType::Issue),
            "bug" => Ok(TaskType::Bug),
            "chore" => Ok(TaskType::Chore),
            // Backward-compatible mapping for legacy persisted values.
            "other" => Ok(TaskType::Chore),
            "refactor" => Ok(TaskType::Refactor),
            other => Err(format!("unknown task type: {other}")),
        }
    }
}

impl TaskType {
    /// Returns true for task types that trigger friction bounty scoreboard hooks.
    pub fn is_friction(&self) -> bool {
        matches!(self, TaskType::Friction | TaskType::Issue)
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
pub struct Task {
    pub id: OrbitId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<OrbitId>,
    pub title: String,
    pub description: String,
    #[serde(default, alias = "instructions")]
    pub plan: String,
    #[serde(default)]
    pub execution_summary: String,
    pub context_files: Vec<String>,
    #[serde(default)]
    pub workspace_path: Option<String>,
    #[serde(default)]
    pub repo_root: Option<String>,
    #[serde(default)]
    pub assigned_to: Option<String>,
    #[serde(default)]
    pub created_by: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    #[serde(default)]
    pub complexity: Option<TaskComplexity>,
    pub task_type: TaskType,
    #[serde(default)]
    pub pr_number: Option<String>,
    #[serde(default)]
    pub proposed_by: Option<String>,
    /// For `Bug` tasks: the originating task whose implementation introduced the defect.
    #[serde(default)]
    pub source_task_id: Option<String>,
    #[serde(default)]
    pub comments: Vec<TaskComment>,
    #[serde(default)]
    pub history: Vec<TaskHistoryEntry>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_can_transition_to_in_progress() {
        assert!(
            TaskStatus::Rejected
                .validate_transition(TaskStatus::InProgress)
                .is_ok()
        );
    }

    #[test]
    fn rejected_can_still_transition_to_backlog() {
        assert!(
            TaskStatus::Rejected
                .validate_transition(TaskStatus::Backlog)
                .is_ok()
        );
    }

    #[test]
    fn task_complexity_round_trips_all_values() {
        for (raw, expected) in [
            ("low", TaskComplexity::Low),
            ("medium", TaskComplexity::Medium),
            ("hard", TaskComplexity::Hard),
        ] {
            assert_eq!(
                TaskComplexity::from_str(raw).expect("parse complexity"),
                expected
            );
            assert_eq!(expected.to_string(), raw);
            assert_eq!(
                serde_json::from_str::<TaskComplexity>(&format!("\"{raw}\""))
                    .expect("deserialize complexity"),
                expected
            );
        }
    }

    #[test]
    fn task_missing_complexity_deserializes_to_none() {
        let task: Task = serde_json::from_value(serde_json::json!({
            "id": "T20260320-000001",
            "title": "Missing complexity",
            "description": "desc",
            "plan": "plan",
            "execution_summary": "",
            "context_files": [],
            "status": "backlog",
            "priority": "medium",
            "task_type": "task",
            "agent": "codex",
            "model": "gpt-5.4",
            "comments": [],
            "history": [],
            "created_at": "2026-03-20T00:00:00Z",
            "updated_at": "2026-03-20T00:00:00Z"
        }))
        .expect("deserialize task");

        assert_eq!(task.complexity, None);
        assert_eq!(task.agent.as_deref(), Some("codex"));
        assert_eq!(task.model.as_deref(), Some("gpt-5.4"));
    }

    #[test]
    fn task_missing_parent_id_deserializes_to_none() {
        let task: Task = serde_json::from_value(serde_json::json!({
            "id": "T20260320-000001",
            "title": "Missing parent",
            "description": "desc",
            "plan": "plan",
            "execution_summary": "",
            "context_files": [],
            "status": "backlog",
            "priority": "medium",
            "task_type": "task",
            "comments": [],
            "history": [],
            "created_at": "2026-03-20T00:00:00Z",
            "updated_at": "2026-03-20T00:00:00Z"
        }))
        .expect("deserialize task");

        assert_eq!(task.parent_id, None);
    }

    #[test]
    fn task_missing_agent_and_model_deserializes_to_none() {
        let task: Task = serde_json::from_value(serde_json::json!({
            "id": "T20260320-000002",
            "title": "Missing agent metadata",
            "description": "desc",
            "plan": "plan",
            "execution_summary": "",
            "context_files": [],
            "status": "backlog",
            "priority": "medium",
            "task_type": "task",
            "comments": [],
            "history": [],
            "created_at": "2026-03-20T00:00:00Z",
            "updated_at": "2026-03-20T00:00:00Z"
        }))
        .expect("deserialize task");

        assert_eq!(task.agent, None);
        assert_eq!(task.model, None);
    }

    #[test]
    fn bug_type_round_trips_via_display_and_from_str() {
        assert_eq!(TaskType::Bug.to_string(), "bug");
        assert_eq!(TaskType::from_str("bug").unwrap(), TaskType::Bug);
        assert_ne!(TaskType::from_str("bug").unwrap(), TaskType::Issue);
    }

    #[test]
    fn bug_type_deserializes_from_json() {
        let task: Task = serde_json::from_value(serde_json::json!({
            "id": "T20260322-000003",
            "title": "Regression in login flow",
            "description": "desc",
            "plan": "",
            "execution_summary": "",
            "context_files": [],
            "status": "backlog",
            "priority": "high",
            "task_type": "bug",
            "agent": "claude",
            "model": "opus-4.6",
            "source_task_id": "T20260320-021158",
            "comments": [],
            "history": [],
            "created_at": "2026-03-22T00:00:00Z",
            "updated_at": "2026-03-22T00:00:00Z"
        }))
        .expect("deserialize bug task");

        assert_eq!(task.task_type, TaskType::Bug);
        assert_eq!(task.source_task_id.as_deref(), Some("T20260320-021158"));
        assert_eq!(task.agent.as_deref(), Some("claude"));
    }

    #[test]
    fn source_task_id_defaults_to_none() {
        let task: Task = serde_json::from_value(serde_json::json!({
            "id": "T20260322-000004",
            "title": "Regular task",
            "description": "desc",
            "plan": "",
            "execution_summary": "",
            "context_files": [],
            "status": "backlog",
            "priority": "medium",
            "task_type": "feature",
            "comments": [],
            "history": [],
            "created_at": "2026-03-22T00:00:00Z",
            "updated_at": "2026-03-22T00:00:00Z"
        }))
        .expect("deserialize task");

        assert_eq!(task.source_task_id, None);
    }
}
