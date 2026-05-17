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

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use orbit_common::types::{Task, TaskPriority, TaskStatus, TaskType};

    use super::*;

    fn task() -> Task {
        Task {
            id: "ORB-00000".to_string(),
            title: "Index this".to_string(),
            description: "Task description".to_string(),
            acceptance_criteria: vec!["First criterion".to_string()],
            plan: "Plan body".to_string(),
            execution_summary: "Summary body".to_string(),
            context_files: Vec::new(),
            created_by: None,
            planned_by: None,
            implemented_by: None,
            status: TaskStatus::Backlog,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Chore,
            pr_status: None,
            external_refs: Vec::new(),
            relations: Vec::new(),
            job_run_id: None,
            crew: None,
            tags: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn task_embedding_fields_use_v2_document_names() {
        let field_names = task_embedding_fields(&task())
            .into_iter()
            .map(|field| field.field)
            .collect::<Vec<_>>();

        assert_eq!(
            field_names,
            vec![
                "title",
                "description",
                "plan",
                "execution_summary",
                "acceptance",
            ]
        );
    }
}
