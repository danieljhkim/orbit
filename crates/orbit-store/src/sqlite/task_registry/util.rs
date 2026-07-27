use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use orbit_common::types::{TaskRelationType, TaskStatus};

pub(super) fn normalize_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

pub(super) fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub(super) fn now_string() -> String {
    Utc::now().to_rfc3339()
}

pub(super) fn terminal_month(status: TaskStatus, updated_at: DateTime<Utc>) -> Option<String> {
    matches!(
        status,
        TaskStatus::Done | TaskStatus::Archived | TaskStatus::Rejected
    )
    .then(|| updated_at.format("%Y-%m").to_string())
}

pub(super) fn relation_type_name(relation_type: TaskRelationType) -> &'static str {
    match relation_type {
        TaskRelationType::BlockedBy => "blocked_by",
        TaskRelationType::ChildOf => "child_of",
        TaskRelationType::SpawnedFrom => "spawned_from",
        TaskRelationType::RegressionFrom => "regression_from",
        TaskRelationType::Supersedes => "supersedes",
        TaskRelationType::RelatedTo => "related_to",
        TaskRelationType::Produces => "produces",
        TaskRelationType::Resolves => "resolves",
    }
}

pub(super) fn parse_relation_type_name(raw: &str) -> Result<TaskRelationType, String> {
    match raw {
        "blocked_by" => Ok(TaskRelationType::BlockedBy),
        "child_of" => Ok(TaskRelationType::ChildOf),
        "spawned_from" => Ok(TaskRelationType::SpawnedFrom),
        "regression_from" => Ok(TaskRelationType::RegressionFrom),
        "supersedes" => Ok(TaskRelationType::Supersedes),
        "related_to" => Ok(TaskRelationType::RelatedTo),
        "produces" => Ok(TaskRelationType::Produces),
        "resolves" => Ok(TaskRelationType::Resolves),
        other => Err(format!("unknown task relation type '{other}'")),
    }
}
