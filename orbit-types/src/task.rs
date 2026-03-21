use std::fmt::{Display, Formatter};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::OrbitId;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Proposed,
    Backlog,
    #[cfg_attr(feature = "clap", value(name = "in-progress", alias = "in_progress"))]
    InProgress,
    Review,
    Done,
    Blocked,
    Archived,
    Rejected,
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
        }
    }

    pub fn validate_transition(&self, target: TaskStatus) -> Result<(), String> {
        if target == TaskStatus::Archived {
            return Ok(());
        }
        if target == TaskStatus::Blocked
            && *self != TaskStatus::Archived
            && *self != TaskStatus::Rejected
        {
            return Ok(());
        }

        let allowed = match self {
            TaskStatus::Proposed => target == TaskStatus::Backlog || target == TaskStatus::Rejected,
            TaskStatus::Backlog => target == TaskStatus::InProgress,
            TaskStatus::InProgress => target == TaskStatus::Review,
            TaskStatus::Review => target == TaskStatus::Done || target == TaskStatus::Rejected,
            TaskStatus::Done => false,
            TaskStatus::Blocked => {
                target == TaskStatus::Backlog || target == TaskStatus::InProgress
            }
            TaskStatus::Archived => target == TaskStatus::Backlog,
            TaskStatus::Rejected => {
                target == TaskStatus::Backlog || target == TaskStatus::InProgress
            }
        };

        if allowed {
            Ok(())
        } else {
            Err(format!("invalid status transition: {} -> {}", self, target))
        }
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
    #[cfg_attr(feature = "clap", value(alias = "bug"))]
    Issue,
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
            TaskType::Issue => "issue",
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
            "issue" => Ok(TaskType::Issue),
            "bug" => Ok(TaskType::Issue),
            "chore" => Ok(TaskType::Chore),
            // Backward-compatible mapping for legacy persisted values.
            "other" => Ok(TaskType::Chore),
            "refactor" => Ok(TaskType::Refactor),
            other => Err(format!("unknown task type: {other}")),
        }
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
    pub status: TaskStatus,
    pub priority: TaskPriority,
    #[serde(default)]
    pub complexity: Option<TaskComplexity>,
    pub task_type: TaskType,
    #[serde(default)]
    pub pr_number: Option<String>,
    #[serde(default)]
    pub proposed_by: Option<String>,
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
            "comments": [],
            "history": [],
            "created_at": "2026-03-20T00:00:00Z",
            "updated_at": "2026-03-20T00:00:00Z"
        }))
        .expect("deserialize task");

        assert_eq!(task.complexity, None);
    }
}
