//! Maps a `Task` to the per-field rows that get embedded individually.
//!
//! Per ADR-003, each task is indexed as multiple rows (purpose, summary, plan,
//! execution_summary, acceptance_criteria, comment_<idx>, review_<thread>_msg_<idx>)
//! so the best-matching field can surface as the snippet at search time.

use orbit_common::types::Task;

use super::EmbeddingField;

pub fn task_embedding_fields(task: &Task) -> Vec<EmbeddingField> {
    let mut fields = Vec::new();
    push_field(&mut fields, "purpose", &task.title);
    push_field(&mut fields, "summary", &task.description);
    push_field(&mut fields, "plan", &task.plan);
    push_field(&mut fields, "execution_summary", &task.execution_summary);
    if !task.acceptance_criteria.is_empty() {
        push_field(
            &mut fields,
            "acceptance_criteria",
            &task.acceptance_criteria.join("\n"),
        );
    }
    for (idx, comment) in task.comments.iter().enumerate() {
        push_field(&mut fields, format!("comment_{idx}"), &comment.message);
    }
    for thread in &task.review_threads {
        for (idx, message) in thread.messages.iter().enumerate() {
            push_field(
                &mut fields,
                format!("review_{}_msg_{idx}", thread.thread_id),
                &message.body,
            );
        }
    }
    fields
}

fn push_field(fields: &mut Vec<EmbeddingField>, field: impl Into<String>, text: &str) {
    if !text.trim().is_empty() {
        fields.push(EmbeddingField::new(field, text.trim().to_string()));
    }
}
