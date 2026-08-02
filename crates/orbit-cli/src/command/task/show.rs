use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime, TaskRelatedDoc};
use serde_json::Value;

use crate::command::{Block, CommandOut, CommandOutput, Execute, Payload};

use super::output::{
    is_human_visible_history_event, print_task_fields, task_fields_to_json,
    task_to_json_for_runtime,
};

#[derive(Args)]
pub struct TaskShowArgs {
    /// Task ID
    pub id: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
    /// Print only the specified field projection(s). Valid values: comments, plan,
    /// execution_summary, description, acceptance_criteria, dependencies,
    /// resolved_dependencies, tags, history, context_files, crew, orchestrator,
    /// artifacts.
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
    fn execute(self, runtime: &OrbitRuntime) -> CommandOut {
        let task = runtime.get_task(&self.id)?;
        let status_by_id = runtime.task_status_index()?;
        let fields = normalize_task_show_fields(&self.fields)?;

        if !fields.is_empty() {
            if self.with_context {
                return Err(OrbitError::InvalidInput(
                    "`--with-context` cannot be combined with `--fields`".to_string(),
                ));
            }
            if self.json {
                return Ok(Payload::document(task_fields_to_json(
                    runtime,
                    &task,
                    &fields,
                    Some(&status_by_id),
                )?)
                .into());
            }
            return {
                print_task_fields(runtime, &task, &fields, Some(&status_by_id))?;
                Ok(CommandOutput::Silent)
            };
        }

        let related_docs = if self.with_context {
            runtime.related_docs_for_task(&task, self.max_docs)?
        } else {
            Vec::new()
        };
        let mut doc = task_to_json_for_runtime(runtime, &task)?;
        if self.with_context {
            insert_related_docs(&mut doc, related_docs.clone())?;
        }

        {
            use std::fmt::Write as _;

            let mut blocks: Vec<Block> = Vec::new();
            let mut out = String::new();
            use crate::output::color::{Domain, bold, dimmed, text};
            let _ = writeln!(out, "{} {}", bold("ID:"), task.id);
            if let Some(parent_id) = task.parent_id() {
                let _ = writeln!(out, "{} {}", bold("Parent Task:"), parent_id);
            }
            let _ = writeln!(out, "{} {}", bold("Title:"), task.title);
            let _ = writeln!(
                out,
                "{} {}",
                bold("Status:"),
                text(&task.status.to_string(), Domain::TaskStatus)
            );
            let _ = writeln!(
                out,
                "{} {}",
                bold("Priority:"),
                text(&task.priority.to_string(), Domain::Priority)
            );
            if let Some(complexity) = task.complexity {
                let _ = writeln!(out, "{} {}", bold("Complexity:"), complexity);
            }
            let _ = writeln!(out, "{} {}", bold("Type:"), task.task_type);
            if !task.description.is_empty() {
                let _ = writeln!(out, "{} {}", bold("Description:"), task.description);
            }
            if !task.acceptance_criteria.is_empty() {
                let _ = writeln!(out, "{}", bold("Acceptance Criteria:"));
                for criterion in &task.acceptance_criteria {
                    let _ = writeln!(out, "  - {}", criterion);
                }
            }
            if !task.dependencies().is_empty() {
                let _ = writeln!(out, "{}", bold("Dependencies:"));
                for dependency in orbit_core::resolve_task_dependencies(&task, &status_by_id) {
                    let _ = writeln!(out, "  - {}", dependency.label());
                }
            }
            if !task.tags.is_empty() {
                let _ = writeln!(out, "{} {}", bold("Tags:"), task.tags.join(", "));
            }
            if !task.external_refs.is_empty() {
                let _ = writeln!(out, "{}", bold("External refs:"));
                for external_ref in &task.external_refs {
                    if let Some(url) = &external_ref.url {
                        let _ = writeln!(
                            out,
                            "  - {}: {} [{}]",
                            external_ref.system, external_ref.id, url
                        );
                    } else {
                        let _ = writeln!(out, "  - {}: {}", external_ref.system, external_ref.id);
                    }
                }
            }
            if !task.plan.is_empty() {
                let _ = writeln!(out, "{} {}", bold("Plan:"), task.plan);
            }
            if !task.execution_summary.is_empty() {
                let _ = writeln!(
                    out,
                    "{} {}",
                    bold("Execution Summary:"),
                    task.execution_summary
                );
            }
            let comments = runtime.get_task_comments(&task.id)?;
            if !comments.is_empty() {
                let _ = writeln!(out, "{}", bold("Comments:"));
                for comment in &comments {
                    let _ = writeln!(
                        out,
                        "  {} {}: {}",
                        dimmed(&format!("[{}]", comment.at.to_rfc3339())),
                        comment.by,
                        comment.message
                    );
                }
            }
            if !task.context_files.is_empty() {
                let _ = writeln!(
                    out,
                    "{} {}",
                    bold("Context:"),
                    task.context_files.join(", ")
                );
            }
            if self.with_context && !related_docs.is_empty() {
                blocks.push(Block::text(std::mem::take(&mut out)));
                blocks.extend(related_docs_blocks(&related_docs));
            }
            if let Some(ref created_by) = task.created_by {
                let _ = writeln!(out, "{} {}", bold("Created By:"), created_by);
            }
            if let Some(ref planned_by) = task.planned_by {
                let _ = writeln!(out, "{} {}", bold("Planned By:"), planned_by);
            }
            if let Some(ref implemented_by) = task.implemented_by {
                let _ = writeln!(out, "{} {}", bold("Implemented By:"), implemented_by);
            }
            if let Some(ref crew) = task.crew {
                let _ = writeln!(out, "{} {}", bold("Execution Crew:"), crew);
            }
            if let Some(ref orchestrator) = task.orchestrator {
                let _ = writeln!(out, "{} {}", bold("Orchestrator:"), orchestrator);
            }
            let history = runtime.get_task_history(&task.id)?;
            let visible_history: Vec<_> = history
                .iter()
                .filter(|entry| is_human_visible_history_event(&entry.event))
                .collect();
            if !visible_history.is_empty() {
                let _ = writeln!(out, "{}", bold("History:"));
                for entry in visible_history {
                    if let Some(note) = &entry.note {
                        let _ = writeln!(
                            out,
                            "  {} {}: {} ({})",
                            dimmed(&format!("[{}]", entry.at.to_rfc3339())),
                            entry.by,
                            entry.event,
                            note
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "  {} {}: {}",
                            dimmed(&format!("[{}]", entry.at.to_rfc3339())),
                            entry.by,
                            entry.event
                        );
                    }
                }
            }
            if let Some(ref pr_status) = task.pr_status {
                let _ = writeln!(out, "{} {}", bold("PR Status:"), pr_status);
            }
            if let Some(source_task_id) = task.source_task_id() {
                let _ = writeln!(out, "{} {}", bold("Source Task:"), source_task_id);
            }
            let _ = writeln!(
                out,
                "{} {}",
                bold("Created:"),
                dimmed(&task.created_at.to_rfc3339())
            );
            let _ = writeln!(
                out,
                "{} {}",
                bold("Updated:"),
                dimmed(&task.updated_at.to_rfc3339())
            );
            blocks.push(Block::text(out));
            Ok(Payload::blocks(doc, blocks).into())
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

fn related_docs_blocks(related_docs: &[TaskRelatedDoc]) -> Vec<Block> {
    use crate::output::color::bold;
    use comfy_table::Cell;

    use crate::output::table::{Column, Table};
    // Part of a detail view: keep every column, and point at `orbit docs show
    // <path>` for the untruncated doc.
    let mut table = Table::new(vec![
        Column::new("PATH").path(),
        Column::new("TYPE").fixed(),
        Column::new("SUMMARY"),
        Column::new("EXCERPT"),
    ])
    .keep_all_columns()
    .empty_message("no related docs");
    for doc in related_docs {
        table.add_row(vec![
            Cell::new(&doc.path),
            Cell::new(doc.doc_type.to_string()),
            Cell::new(&doc.summary),
            Cell::new(&doc.excerpt),
        ]);
    }
    vec![
        Block::text(format!("\n{}", bold("Related Docs:"))),
        Block::table(table),
    ]
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
                | "crew"
                | "orchestrator"
                | "artifacts"
        ) {
            return Err(OrbitError::InvalidInput(format!(
                "unknown field selector `{trimmed}`. Valid values: comments, plan, execution_summary, description, acceptance_criteria, dependencies, resolved_dependencies, tags, history, context_files, crew, orchestrator, artifacts"
            )));
        }
        normalized.push(trimmed.to_string());
    }
    Ok(normalized)
}
