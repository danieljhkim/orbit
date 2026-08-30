//! Canonical `orbit.task.show` field-projection vocabulary.
//!
//! CLI help, MCP/tool schema text, validation errors, and JSON projectors
//! must all draw from this list. Sidecar fields (`comments`, `history`,
//! `artifacts`) and `resolved_dependencies` still need a runtime fetch;
//! the Task-local scalars below are projected from the record itself.

use serde_json::{Value, json};

use crate::task::Task;

/// String-literal CSV of [`TASK_SHOW_PROJECTION_FIELDS`], for `concat!` in
/// clap help and other const contexts.
#[macro_export]
macro_rules! task_show_projection_fields_csv {
    () => {
        "id, title, type, status, priority, complexity, created_at, updated_at, description, acceptance_criteria, dependencies, resolved_dependencies, tags, required_tools, plan, execution_summary, context_files, crew, orchestrator, comments, history, artifacts"
    };
}

/// Authoritative `orbit.task.show` `--fields` / `fields` / `field` vocabulary.
pub const TASK_SHOW_PROJECTION_FIELDS: &[&str] = &[
    "id",
    "title",
    "type",
    "status",
    "priority",
    "complexity",
    "created_at",
    "updated_at",
    "description",
    "acceptance_criteria",
    "dependencies",
    "resolved_dependencies",
    "tags",
    "required_tools",
    "plan",
    "execution_summary",
    "context_files",
    "crew",
    "orchestrator",
    "comments",
    "history",
    "artifacts",
];

/// Comma-separated form of [`TASK_SHOW_PROJECTION_FIELDS`].
pub const TASK_SHOW_PROJECTION_FIELDS_CSV: &str = crate::task_show_projection_fields_csv!();

/// Whether `name` is in the canonical show-projection vocabulary.
pub fn is_task_show_projection_field(name: &str) -> bool {
    TASK_SHOW_PROJECTION_FIELDS.contains(&name)
}

/// Actionable error for a name that is not in the vocabulary.
pub fn unknown_task_show_field_message(name: &str) -> String {
    format!("unknown field selector `{name}`. Valid values: {TASK_SHOW_PROJECTION_FIELDS_CSV}")
}

/// JSON for a Task-local (non-sidecar) projection field.
///
/// Returns `None` for sidecar names, `resolved_dependencies`, and unknown
/// names so callers can keep those on their existing fetch paths.
pub fn task_show_record_field_json(task: &Task, field: &str) -> Option<Value> {
    match field {
        "id" => Some(json!(task.id)),
        "title" => Some(json!(task.title)),
        "type" => Some(json!(task.task_type.to_string())),
        "status" => Some(json!(task.status.to_string())),
        "priority" => Some(json!(task.priority.to_string())),
        "complexity" => Some(json!(task.complexity.map(|value| value.to_string()))),
        "created_at" => Some(json!(task.created_at.to_rfc3339())),
        "updated_at" => Some(json!(task.updated_at.to_rfc3339())),
        _ => None,
    }
}
