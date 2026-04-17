use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::output::{print_task_fields, task_fields_to_json, task_to_json};

#[derive(Args)]
pub struct TaskShowArgs {
    /// Task ID
    pub id: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
    /// Print only the specified field projection(s). Valid values: comments, plan,
    /// execution_summary, description, acceptance_criteria, history, context_files, artifacts.
    /// Repeat the flag or use a comma-separated value list. Combined with --json,
    /// a single field returns that field as JSON and multiple fields return a JSON object.
    #[arg(long = "fields", alias = "field", value_delimiter = ',', num_args = 1..)]
    pub fields: Vec<String>,
}

impl Execute for TaskShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let task = runtime.get_task(&self.id)?;
        let fields = normalize_task_show_fields(&self.fields)?;

        if !fields.is_empty() {
            if self.json {
                return crate::output::json::print_pretty(&task_fields_to_json(
                    runtime, &task, &fields,
                )?);
            }
            return print_task_fields(runtime, &task, &fields);
        }

        if self.json {
            crate::output::json::print_pretty(&task_to_json(&task))
        } else {
            use crate::output::color::{bold, dimmed, priority_color, status_color};
            println!("{} {}", bold("ID:"), task.id);
            if let Some(ref parent_id) = task.parent_id {
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
            if !task.plan.is_empty() {
                println!("{} {}", bold("Plan:"), task.plan);
            }
            if !task.execution_summary.is_empty() {
                println!("{} {}", bold("Execution Summary:"), task.execution_summary);
            }
            if !task.comments.is_empty() {
                println!("{}", bold("Comments:"));
                for comment in &task.comments {
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
            if let Some(ref created_by) = task.created_by {
                println!("{} {}", bold("Created By:"), created_by);
            }
            if let Some(ref planned_by) = task.planned_by {
                println!("{} {}", bold("Planned By:"), planned_by);
            }
            if let Some(ref implemented_by) = task.implemented_by {
                println!("{} {}", bold("Implemented By:"), implemented_by);
            }
            if !task.history.is_empty() {
                println!("{}", bold("History:"));
                for entry in &task.history {
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
            if let Some(ref pr_number) = task.pr_number {
                println!("{} {}", bold("PR Number:"), pr_number);
            }
            if let Some(ref pr_status) = task.pr_status {
                println!("{} {}", bold("PR Status:"), pr_status);
            }
            if let Some(ref source_task_id) = task.source_task_id {
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
                | "history"
                | "context_files"
                | "artifacts"
        ) {
            return Err(OrbitError::InvalidInput(format!(
                "unknown field selector `{trimmed}`. Valid values: comments, plan, execution_summary, description, acceptance_criteria, history, context_files, artifacts"
            )));
        }
        normalized.push(trimmed.to_string());
    }
    Ok(normalized)
}
