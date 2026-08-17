use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

const DEFAULT_CHECKPOINT_ATTEMPT_BUDGET: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(try_from = "TaskPlanDocument", into = "TaskPlanDocument")]
pub struct TaskPlan {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checkpoints: Vec<TaskPlanCheckpoint>,
}

impl TaskPlan {
    pub fn is_empty(&self) -> bool {
        self.checkpoints.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    try_from = "TaskPlanCheckpointDocument",
    into = "TaskPlanCheckpointDocument"
)]
pub struct TaskPlanCheckpoint {
    pub id: String,
    pub spec: String,
    pub success_criteria: Vec<TaskPlanSuccessCriterion>,
    pub attempt_budget: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    try_from = "TaskPlanSuccessCriterionDocument",
    into = "TaskPlanSuccessCriterionDocument"
)]
pub enum TaskPlanSuccessCriterion {
    Command { command: String, expect_exit: i32 },
    FileExists { path: String },
    FileContains { path: String, pattern: String },
    Semantic { statement: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
struct TaskPlanDocument {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    checkpoints: Vec<TaskPlanCheckpoint>,
}

impl TryFrom<TaskPlanDocument> for TaskPlan {
    type Error = String;

    fn try_from(value: TaskPlanDocument) -> Result<Self, Self::Error> {
        let mut checkpoint_ids = BTreeSet::new();
        for checkpoint in &value.checkpoints {
            if !checkpoint_ids.insert(checkpoint.id.clone()) {
                return Err(format!("duplicate checkpoint id '{}'", checkpoint.id));
            }
        }

        Ok(Self {
            checkpoints: value.checkpoints,
        })
    }
}

impl From<TaskPlan> for TaskPlanDocument {
    fn from(value: TaskPlan) -> Self {
        Self {
            checkpoints: value.checkpoints,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
struct TaskPlanCheckpointDocument {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    spec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    success_criteria: Option<Vec<TaskPlanSuccessCriterion>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    attempt_budget: Option<u32>,
}

impl TryFrom<TaskPlanCheckpointDocument> for TaskPlanCheckpoint {
    type Error = String;

    fn try_from(value: TaskPlanCheckpointDocument) -> Result<Self, Self::Error> {
        Ok(Self {
            id: require_string(value.id, "id", "checkpoint")?,
            spec: require_string(value.spec, "spec", "checkpoint")?,
            success_criteria: value.success_criteria.ok_or_else(|| {
                "checkpoint missing required field 'success_criteria'".to_string()
            })?,
            attempt_budget: value
                .attempt_budget
                .unwrap_or(DEFAULT_CHECKPOINT_ATTEMPT_BUDGET),
        })
    }
}

impl From<TaskPlanCheckpoint> for TaskPlanCheckpointDocument {
    fn from(value: TaskPlanCheckpoint) -> Self {
        Self {
            id: Some(value.id),
            spec: Some(value.spec),
            success_criteria: Some(value.success_criteria),
            attempt_budget: (value.attempt_budget != DEFAULT_CHECKPOINT_ATTEMPT_BUDGET)
                .then_some(value.attempt_budget),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TaskPlanSuccessCriterionDocument {
    Command {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expect_exit: Option<i32>,
    },
    FileExists {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    FileContains {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pattern: Option<String>,
    },
    Semantic {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        statement: Option<String>,
    },
}

impl TryFrom<TaskPlanSuccessCriterionDocument> for TaskPlanSuccessCriterion {
    type Error = String;

    fn try_from(value: TaskPlanSuccessCriterionDocument) -> Result<Self, Self::Error> {
        match value {
            TaskPlanSuccessCriterionDocument::Command {
                command,
                expect_exit,
            } => Ok(Self::Command {
                command: require_string(command, "command", "criterion kind 'command'")?,
                expect_exit: expect_exit.ok_or_else(|| {
                    "criterion kind 'command' missing required field 'expect_exit'".to_string()
                })?,
            }),
            TaskPlanSuccessCriterionDocument::FileExists { path } => Ok(Self::FileExists {
                path: require_string(path, "path", "criterion kind 'file_exists'")?,
            }),
            TaskPlanSuccessCriterionDocument::FileContains { path, pattern } => {
                Ok(Self::FileContains {
                    path: require_string(path, "path", "criterion kind 'file_contains'")?,
                    pattern: require_string(pattern, "pattern", "criterion kind 'file_contains'")?,
                })
            }
            TaskPlanSuccessCriterionDocument::Semantic { statement } => Ok(Self::Semantic {
                statement: require_string(statement, "statement", "criterion kind 'semantic'")?,
            }),
        }
    }
}

impl From<TaskPlanSuccessCriterion> for TaskPlanSuccessCriterionDocument {
    fn from(value: TaskPlanSuccessCriterion) -> Self {
        match value {
            TaskPlanSuccessCriterion::Command {
                command,
                expect_exit,
            } => Self::Command {
                command: Some(command),
                expect_exit: Some(expect_exit),
            },
            TaskPlanSuccessCriterion::FileExists { path } => Self::FileExists { path: Some(path) },
            TaskPlanSuccessCriterion::FileContains { path, pattern } => Self::FileContains {
                path: Some(path),
                pattern: Some(pattern),
            },
            TaskPlanSuccessCriterion::Semantic { statement } => Self::Semantic {
                statement: Some(statement),
            },
        }
    }
}

fn require_string(value: Option<String>, field: &str, context: &str) -> Result<String, String> {
    let value = value.ok_or_else(|| format!("{context} missing required field '{field}'"))?;
    if value.trim().is_empty() {
        return Err(format!("{context} requires non-empty field '{field}'"));
    }
    Ok(value)
}
