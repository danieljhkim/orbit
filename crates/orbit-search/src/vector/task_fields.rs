//! Maps a `Task` to the per-field rows that get embedded individually.
//!
//! Each task is indexed as multiple rows whose field names match the logical
//! v2 task documents where possible (title, description, acceptance, plan,
//! execution_summary, comment_<idx>, review_<thread>_msg_<idx>) so the
//! best-matching field can surface as the snippet at search time.

use orbit_common::types::Task;

use super::EmbeddingField;

pub fn task_embedding_fields(task: &Task) -> Vec<EmbeddingField> {
    let mut fields = Vec::new();
    push_field(&mut fields, "title", &task.title);
    push_field(&mut fields, "description", &task.description);
    push_field(&mut fields, "plan", &task.plan);
    push_field(&mut fields, "execution_summary", &task.execution_summary);
    if !task.acceptance_criteria.is_empty() {
        push_field(
            &mut fields,
            "acceptance",
            &task.acceptance_criteria.join("\n"),
        );
    }
    fields
}

fn push_field(fields: &mut Vec<EmbeddingField>, field: impl Into<String>, text: &str) {
    if !text.trim().is_empty() {
        fields.push(EmbeddingField::new(field, text.trim().to_string()));
    }
}
