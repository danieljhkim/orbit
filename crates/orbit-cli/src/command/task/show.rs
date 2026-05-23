use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime, TaskRelatedDoc, build_task_status_index};
use serde_json::Value;

use crate::command::Execute;

use super::output::{print_task_fields, task_fields_to_json, task_to_json_for_runtime};

#[derive(Args)]
pub struct TaskShowArgs {
    /// Task ID
    pub id: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
    /// Print only the specified field projection(s). Valid values: comments, plan,
    /// execution_summary, description, acceptance_criteria, dependencies,
    /// resolved_dependencies, tags, history, context_files, artifacts.
    /// Repeat the flag or use a comma-separated value list. Combined with --json,
    /// a single field returns that field as JSON and multiple fields return a JSON object.
    #[arg(long = "fields", alias = "field", value_delimiter = ',', num_args = 1..)]
    pub fields: Vec<String>,
    /// Include docs matched from task context files and task feature tags
    #[arg(long)]
    pub with_context: bool,
    /// Maximum related docs to include with --with-context (default 5)
    #[arg(long)]
    pub max_docs: Option<usize>,
}

impl Execute for TaskShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let task = runtime.get_task(&self.id)?;
        let status_by_id = build_task_status_index(&runtime.list_tasks()?);
        let fields = normalize_task_show_fields(&self.fields)?;

        if !fields.is_empty() {
            if self.with_context {
                return Err(OrbitError::InvalidInput(
                    "`--with-context` cannot be combined with `--fields`".to_string(),
                ));
            }
            if self.json {
                return crate::output::json::print_pretty(&task_fields_to_json(
                    runtime,
                    &task,
                    &fields,
                    Some(&status_by_id),
                )?);
            }
            return print_task_fields(runtime, &task, &fields, Some(&status_by_id));
        }

        let related_docs = if self.with_context {
            runtime.related_docs_for_task(&task, self.max_docs)?
        } else {
            Vec::new()
        };
        if self.json {
            let mut value = task_to_json_for_runtime(runtime, &task)?;
            if self.with_context {
                insert_related_docs(&mut value, related_docs)?;
            }
            crate::output::json::print_pretty(&value)
        } else {
            use crate::output::color::{bold, dimmed, priority_color, status_color};
            println!("{} {}", bold("ID:"), task.id);
            if let Some(parent_id) = task.parent_id() {
                println!("{} {}", bold("Parent Task:"), parent_id);
            }
            println!("{} {}", bold("Title:"), task.title);
            println!(
                "{} {}",
                bold("Status:"),
                status_color(&task.status.to_string())
            );
            println!(
                "{} {}",
                bold("Priority:"),
                priority_color(&task.priority.to_string())
            );
            if let Some(complexity) = task.complexity {
                println!("{} {}", bold("Complexity:"), complexity);
            }
            println!("{} {}", bold("Type:"), task.task_type);
            if !task.description.is_empty() {
                println!("{} {}", bold("Description:"), task.description);
            }
            if !task.acceptance_criteria.is_empty() {
                println!("{}", bold("Acceptance Criteria:"));
                for criterion in &task.acceptance_criteria {
                    println!("  - {}", criterion);
                }
            }
            if !task.dependencies().is_empty() {
                println!("{}", bold("Dependencies:"));
                for dependency in orbit_core::resolve_task_dependencies(&task, &status_by_id) {
                    println!("  - {}", dependency.label());
                }
            }
            if !task.tags.is_empty() {
                println!("{} {}", bold("Tags:"), task.tags.join(", "));
            }
            if !task.external_refs.is_empty() {
                println!("{}", bold("External refs:"));
                for external_ref in &task.external_refs {
                    if let Some(url) = &external_ref.url {
                        println!("  - {}: {} [{}]", external_ref.system, external_ref.id, url);
                    } else {
                        println!("  - {}: {}", external_ref.system, external_ref.id);
                    }
                }
            }
            if !task.plan.is_empty() {
                println!("{} {}", bold("Plan:"), task.plan);
            }
            if !task.execution_summary.is_empty() {
                println!("{} {}", bold("Execution Summary:"), task.execution_summary);
            }
            let comments = runtime.get_task_comments(&task.id)?;
            if !comments.is_empty() {
                println!("{}", bold("Comments:"));
                for comment in &comments {
                    println!(
                        "  {} {}: {}",
                        dimmed(&format!("[{}]", comment.at.to_rfc3339())),
                        comment.by,
                        comment.message
                    );
                }
            }
            if !task.context_files.is_empty() {
                println!("{} {}", bold("Context:"), task.context_files.join(", "));
            }
            if self.with_context && !related_docs.is_empty() {
                print_related_docs(&related_docs);
            }
            if let Some(ref created_by) = task.created_by {
                println!("{} {}", bold("Created By:"), created_by);
            }
            if let Some(ref planned_by) = task.planned_by {
                println!("{} {}", bold("Planned By:"), planned_by);
            }
            if let Some(ref implemented_by) = task.implemented_by {
                println!("{} {}", bold("Implemented By:"), implemented_by);
            }
            let history = runtime.get_task_history(&task.id)?;
            if !history.is_empty() {
                println!("{}", bold("History:"));
                for entry in &history {
                    if let Some(note) = &entry.note {
                        println!(
                            "  {} {}: {} ({})",
                            dimmed(&format!("[{}]", entry.at.to_rfc3339())),
                            entry.by,
                            entry.event,
                            note
                        );
                    } else {
                        println!(
                            "  {} {}: {}",
                            dimmed(&format!("[{}]", entry.at.to_rfc3339())),
                            entry.by,
                            entry.event
                        );
                    }
                }
            }
            if let Some(ref pr_status) = task.pr_status {
                println!("{} {}", bold("PR Status:"), pr_status);
            }
            if let Some(source_task_id) = task.source_task_id() {
                println!("{} {}", bold("Source Task:"), source_task_id);
            }
            println!(
                "{} {}",
                bold("Created:"),
                dimmed(&task.created_at.to_rfc3339())
            );
            println!(
                "{} {}",
                bold("Updated:"),
                dimmed(&task.updated_at.to_rfc3339())
            );
            Ok(())
        }
    }
}

fn insert_related_docs(
    value: &mut Value,
    related_docs: Vec<TaskRelatedDoc>,
) -> Result<(), OrbitError> {
    let object = value.as_object_mut().ok_or_else(|| {
        OrbitError::Execution("task JSON projection did not produce an object".to_string())
    })?;
    object.insert(
        "related_docs".to_string(),
        serde_json::to_value(related_docs).map_err(|error| {
            OrbitError::Execution(format!("serialize related docs output: {error}"))
        })?,
    );
    Ok(())
}

fn print_related_docs(related_docs: &[TaskRelatedDoc]) {
    use crate::output::color::bold;
    use comfy_table::Cell;

    println!();
    println!("{}", bold("Related Docs:"));
    let mut table = crate::output::table::build_table(&["PATH", "TYPE", "SUMMARY", "EXCERPT"]);
    for doc in related_docs {
        table.add_row(vec![
            Cell::new(&doc.path),
            Cell::new(doc.doc_type.to_string()),
            Cell::new(&doc.summary),
            Cell::new(&doc.excerpt),
        ]);
    }
    println!("{table}");
}

fn normalize_task_show_fields(fields: &[String]) -> Result<Vec<String>, OrbitError> {
    let mut normalized = Vec::new();
    for field in fields {
        let trimmed = field.trim();
        if trimmed.is_empty() {
            return Err(OrbitError::InvalidInput(
                "task show field selectors must not be empty".to_string(),
            ));
        }
        if !matches!(
            trimmed,
            "comments"
                | "plan"
                | "execution_summary"
                | "description"
                | "acceptance_criteria"
                | "dependencies"
                | "resolved_dependencies"
                | "tags"
                | "history"
                | "context_files"
                | "artifacts"
        ) {
            return Err(OrbitError::InvalidInput(format!(
                "unknown field selector `{trimmed}`. Valid values: comments, plan, execution_summary, description, acceptance_criteria, dependencies, resolved_dependencies, tags, history, context_files, artifacts"
            )));
        }
        normalized.push(trimmed.to_string());
    }
    Ok(normalized)
}
