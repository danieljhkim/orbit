use orbit_common::types::{Task, TaskType};

const BATCH_SUBJECT_BUDGET: usize = 72;
const ELLIPSIS: char = '…';

pub(super) fn task_commit_message(task: &Task) -> String {
    let mut message = format!("[{}] {}", task.id, task.title.trim());
    if let Some(summary) = execution_summary_paragraph(task) {
        message.push_str("\n\n");
        message.push_str(&summary);
    }
    message
}

pub(super) fn finalize_commit_message(tasks: &[Task]) -> String {
    if tasks.len() == 1 {
        let task = &tasks[0];
        let summary =
            execution_summary_paragraph(task).unwrap_or_else(|| task.title.trim().to_string());
        let subject = single_line_summary(&summary);
        let mut message = format!("fix: {} [{}]", subject, task.id);
        if summary != subject {
            message.push_str("\n\n");
            message.push_str(&summary);
        }
        return message;
    }

    let ids_joined = tasks
        .iter()
        .map(|task| task.id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let summaries = tasks
        .iter()
        .map(|task| {
            let summary =
                execution_summary_paragraph(task).unwrap_or_else(|| task.title.trim().to_string());
            format!("- {}: {}", task.id, single_line_summary(&summary))
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!("fix: finalize ship batch [{ids_joined}]\n\n{summaries}")
}

pub(super) fn batch_commit_message(task: &Task) -> String {
    let commit_type = conventional_commit_type(task.task_type);
    let title = task.title.trim();
    let (subject_title, truncated) = truncate_title_for_subject(commit_type, title);

    let mut subject = format!("{commit_type}: {subject_title} [{}]", task.id);
    for external_ref in &task.external_refs {
        subject.push(' ');
        subject.push_str(&format!(
            "[{}-{}]",
            external_ref.system.to_ascii_uppercase(),
            external_ref.id
        ));
    }

    let mut sections = Vec::new();
    if truncated {
        sections.push(title.to_string());
    }
    if let Some(summary) = execution_summary_paragraph(task) {
        sections.push(summary);
    }
    let trailers = batch_commit_trailers(task);
    if !trailers.is_empty() {
        sections.push(trailers.join("\n"));
    }

    if sections.is_empty() {
        subject
    } else {
        format!("{subject}\n\n{}", sections.join("\n\n"))
    }
}

fn conventional_commit_type(task_type: TaskType) -> &'static str {
    match task_type {
        TaskType::Feature => "feat",
        TaskType::Bug => "fix",
        TaskType::Refactor => "refactor",
        TaskType::Chore => "chore",
    }
}

fn truncate_title_for_subject(commit_type: &str, title: &str) -> (String, bool) {
    let prefix_len = commit_type.chars().count() + ": ".chars().count();
    let title_budget = BATCH_SUBJECT_BUDGET.saturating_sub(prefix_len);
    if title.chars().count() <= title_budget {
        return (title.to_string(), false);
    }

    let retained_chars = title_budget.saturating_sub(1);
    let mut truncated = title.chars().take(retained_chars).collect::<String>();
    truncated.push(ELLIPSIS);
    (truncated, true)
}

fn batch_commit_trailers(task: &Task) -> Vec<String> {
    let mut trailers = Vec::new();
    if let Some(planned_by) = task.planned_by.as_deref() {
        trailers.push(format!("Planned-By: {planned_by}"));
    }
    if let Some(implemented_by) = task.implemented_by.as_deref() {
        trailers.push(format!("Implemented-By: {implemented_by}"));
    }
    trailers
}

fn execution_summary_paragraph(task: &Task) -> Option<String> {
    let section = extract_summary_section(&task.execution_summary)?;
    let paragraph = section
        .lines()
        .map(str::trim)
        .map(|line| {
            line.trim_start_matches("- ")
                .trim_start_matches("* ")
                .trim()
        })
        .skip_while(|line| line.is_empty())
        .take_while(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let paragraph = paragraph.trim();
    (!paragraph.is_empty()).then_some(paragraph.to_string())
}

fn extract_summary_section(summary: &str) -> Option<String> {
    let mut in_section = false;
    let mut lines = Vec::new();

    for line in summary.lines() {
        let trimmed = line.trim();
        let is_heading = trimmed.starts_with("## ");
        if trimmed == "## 1. Summary of Changes" || trimmed == "## Summary" {
            in_section = true;
            continue;
        }
        if in_section && is_heading {
            break;
        }
        if in_section {
            lines.push(trimmed.to_string());
        }
    }

    let section = lines.join("\n");
    let section = section.trim();
    (!section.is_empty()).then_some(section.to_string())
}

fn single_line_summary(summary: &str) -> String {
    summary
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use orbit_common::types::{ExternalRef, TaskPriority, TaskStatus};

    use super::*;

    #[test]
    fn batch_commit_subject_uses_task_type() {
        let cases = [
            (TaskType::Feature, "feat"),
            (TaskType::Bug, "fix"),
            (TaskType::Refactor, "refactor"),
            (TaskType::Chore, "chore"),
        ];

        for (task_type, expected_type) in cases {
            let task = task_with_type(task_type, "Ship better commit messages");

            assert_eq!(
                batch_commit_message(&task),
                format!("{expected_type}: Ship better commit messages [ORB-00107]")
            );
        }
    }

    #[test]
    fn batch_commit_subject_keeps_short_title_unchanged() {
        let task = task_with_type(TaskType::Feature, "Short title");

        assert_eq!(batch_commit_message(&task), "feat: Short title [ORB-00107]");
    }

    #[test]
    fn batch_commit_subject_truncates_long_title_with_ellipsis() {
        let title = "a".repeat(145);
        let task = task_with_type(TaskType::Feature, &title);
        let message = batch_commit_message(&task);
        let subject = message.lines().next().expect("message has subject");
        let typed_subject = subject
            .split(" [")
            .next()
            .expect("subject includes task tag");

        assert_eq!(typed_subject.chars().count(), 72);
        assert_eq!(typed_subject, format!("feat: {}{ELLIPSIS}", "a".repeat(65)));
    }

    #[test]
    fn batch_commit_body_includes_full_title_only_when_truncated() {
        let short_task = task_with_type(TaskType::Chore, "Short title");
        assert_eq!(
            batch_commit_message(&short_task),
            "chore: Short title [ORB-00107]"
        );

        let title = "b".repeat(145);
        let long_task = task_with_type(TaskType::Feature, &title);
        assert_eq!(
            batch_commit_message(&long_task),
            format!(
                "feat: {}{ELLIPSIS} [ORB-00107]\n\n{}",
                "b".repeat(65),
                title
            )
        );
    }

    #[test]
    fn batch_commit_subject_appends_external_refs_in_declaration_order() {
        let mut task = task_with_type(TaskType::Chore, "Wire external refs");
        task.external_refs = vec![
            external_ref("eng", "1234"),
            external_ref("jira", "CORE-987"),
        ];

        assert_eq!(
            batch_commit_message(&task),
            "chore: Wire external refs [ORB-00107] [ENG-1234] [JIRA-CORE-987]"
        );
    }

    #[test]
    fn batch_commit_body_includes_execution_summary_when_present() {
        let mut task = task_with_type(TaskType::Feature, "Summarize the work");
        task.execution_summary =
            "## Summary\n- Added deterministic batch commit messages.\n\n## Validation\n- cargo test"
                .to_string();

        assert_eq!(
            batch_commit_message(&task),
            "feat: Summarize the work [ORB-00107]\n\nAdded deterministic batch commit messages."
        );
    }

    #[test]
    fn batch_commit_body_omits_execution_summary_when_absent() {
        let mut task = task_with_type(TaskType::Feature, "No summary available");
        task.execution_summary = "## Validation\n- cargo test".to_string();

        assert_eq!(
            batch_commit_message(&task),
            "feat: No summary available [ORB-00107]"
        );
    }

    #[test]
    fn batch_commit_body_orders_full_title_before_execution_summary() {
        let title = "c".repeat(145);
        let mut task = task_with_type(TaskType::Bug, &title);
        task.execution_summary = "## Summary\n- Preserved the full task title.".to_string();

        assert_eq!(
            batch_commit_message(&task),
            format!(
                "fix: {}{ELLIPSIS} [ORB-00107]\n\n{}\n\nPreserved the full task title.",
                "c".repeat(66),
                title
            )
        );
    }

    #[test]
    fn batch_commit_trailers_include_raw_planner_and_implementer() {
        let mut task = task_with_type(TaskType::Refactor, "Record attribution");
        task.planned_by = Some("codex".to_string());
        task.implemented_by = Some("claude".to_string());

        assert_eq!(
            batch_commit_message(&task),
            "refactor: Record attribution [ORB-00107]\n\nPlanned-By: codex\nImplemented-By: claude"
        );
    }

    #[test]
    fn batch_commit_trailers_omit_missing_fields() {
        let mut planned_only = task_with_type(TaskType::Refactor, "Planner only");
        planned_only.planned_by = Some("codex".to_string());
        assert_eq!(
            batch_commit_message(&planned_only),
            "refactor: Planner only [ORB-00107]\n\nPlanned-By: codex"
        );

        let mut implemented_only = task_with_type(TaskType::Refactor, "Implementer only");
        implemented_only.implemented_by = Some("claude".to_string());
        assert_eq!(
            batch_commit_message(&implemented_only),
            "refactor: Implementer only [ORB-00107]\n\nImplemented-By: claude"
        );

        let neither = task_with_type(TaskType::Refactor, "No trailers");
        assert_eq!(
            batch_commit_message(&neither),
            "refactor: No trailers [ORB-00107]"
        );
    }

    fn task_with_type(task_type: TaskType, title: &str) -> Task {
        let now = Utc::now();
        Task {
            id: "ORB-00107".to_string(),
            title: title.to_string(),
            description: String::new(),
            acceptance_criteria: Vec::new(),
            tags: Vec::new(),
            plan: String::new(),
            execution_summary: String::new(),
            context_files: Vec::new(),
            created_by: None,
            planned_by: None,
            implemented_by: None,
            status: TaskStatus::InProgress,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type,
            pr_status: None,
            external_refs: Vec::new(),
            relations: Vec::new(),
            job_run_id: None,
            crew: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn external_ref(system: &str, id: &str) -> ExternalRef {
        ExternalRef::try_new(system.to_string(), id.to_string(), None)
            .expect("external ref fixture is valid")
    }
}
