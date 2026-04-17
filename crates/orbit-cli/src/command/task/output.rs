use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use orbit_core::{OrbitError, OrbitRuntime};
use serde_json::{Value, json};

pub(super) fn task_to_signal_json(task: &orbit_core::Task) -> Value {
    json!({
        "id": task.id,
        "parent_id": task.parent_id,
        "title": task.title,
        "type": task.task_type.to_string(),
        "status": task.status.to_string(),
        "priority": task.priority.to_string(),
        "complexity": task.complexity.map(|value| value.to_string()),
    })
}

pub(super) fn task_to_json(task: &orbit_core::Task) -> Value {
    json!({
        "id": task.id,
        "parent_id": task.parent_id,
        "title": task.title,
        "description": task.description,
        "acceptance_criteria": task.acceptance_criteria,
        "plan": task.plan,
        "execution_summary": task.execution_summary,
        "context_files": task.context_files,
        "workspace_path": task.workspace_path,
        "repo_root": task.repo_root,
        "created_by": task.created_by,
        "planned_by": task.planned_by,
        "implemented_by": task.implemented_by,
        "status": task.status.to_string(),
        "priority": task.priority.to_string(),
        "complexity": task.complexity.map(|value| value.to_string()),
        "type": task.task_type.to_string(),
        "pr_number": task.pr_number,
        "pr_status": task.pr_status,
        "source_task_id": task.source_task_id,
        "comments": task.comments,
        "history": task.history,
        "review_threads": task.review_threads,
        "created_at": task.created_at.to_rfc3339(),
        "updated_at": task.updated_at.to_rfc3339(),
    })
}

pub(super) fn task_lock_to_json(task: &orbit_core::Task) -> Value {
    json!({
        "id": task.id,
        "title": task.title,
        "status": task.status.to_string(),
        "batch_id": task.batch_id,
        "context_files": task.context_files,
    })
}

pub(super) fn print_task_table(tasks: &[orbit_core::Task], full: bool) {
    use comfy_table::Cell;
    let headers = if full {
        vec![
            "ID",
            "TITLE",
            "STATUS",
            "PRIORITY",
            "TYPE",
            "IMPLEMENTED_BY",
            "CREATED_AT",
            "UPDATED_AT",
        ]
    } else {
        vec!["ID", "TITLE", "STATUS", "PRIORITY", "TYPE"]
    };
    let mut table = crate::output::table::build_table(&headers);
    for task in tasks {
        let mut row = vec![
            Cell::new(&task.id),
            Cell::new(&task.title),
            crate::output::color::status_color_cell(&task.status.to_string()),
            crate::output::color::priority_color_cell(&task.priority.to_string()),
            Cell::new(task.task_type.to_string()),
        ];
        if full {
            row.extend([
                Cell::new(task.implemented_by.as_deref().unwrap_or("-")),
                Cell::new(format_task_table_timestamp(task.created_at)),
                Cell::new(format_task_table_timestamp(task.updated_at)),
            ]);
        }
        crate::output::table::add_single_line_row(&mut table, row);
    }
    println!("{table}");
}

pub(super) fn print_task_locks(tasks: &[orbit_core::Task], locked_files: &BTreeSet<String>) {
    if tasks.is_empty() {
        println!("No files currently locked.");
        return;
    }

    for (index, task) in tasks.iter().enumerate() {
        if index > 0 {
            println!();
        }

        match task.batch_id.as_deref() {
            Some(batch_id) => println!(
                "[{}] {} ({}, batch={})",
                task.id, task.title, task.status, batch_id
            ),
            None => println!("[{}] {} ({})", task.id, task.title, task.status),
        }

        for path in &task.context_files {
            println!("  - {}", path);
        }
    }

    println!(
        "\n{} file(s) locked across {} task(s).",
        locked_files.len(),
        tasks.len()
    );
}

pub(super) fn task_field_to_json(
    runtime: &OrbitRuntime,
    task: &orbit_core::Task,
    field: &str,
) -> Result<Value, OrbitError> {
    match field {
        "comments" => {
            serde_json::to_value(&task.comments).map_err(|e| OrbitError::Io(e.to_string()))
        }
        "plan" => Ok(Value::String(task.plan.clone())),
        "execution_summary" => Ok(Value::String(task.execution_summary.clone())),
        "description" => Ok(Value::String(task.description.clone())),
        "acceptance_criteria" => serde_json::to_value(&task.acceptance_criteria)
            .map_err(|e| OrbitError::Io(e.to_string())),
        "history" => serde_json::to_value(&task.history).map_err(|e| OrbitError::Io(e.to_string())),
        "context_files" => {
            serde_json::to_value(&task.context_files).map_err(|e| OrbitError::Io(e.to_string()))
        }
        "artifacts" => serde_json::to_value(runtime.get_task_artifacts(&task.id)?)
            .map_err(|e| OrbitError::Io(e.to_string())),
        other => Err(OrbitError::InvalidInput(format!(
            "unknown field selector `{other}`. Valid values: comments, plan, execution_summary, description, acceptance_criteria, history, context_files, artifacts"
        ))),
    }
}

pub(super) fn task_fields_to_json(
    runtime: &OrbitRuntime,
    task: &orbit_core::Task,
    fields: &[String],
) -> Result<Value, OrbitError> {
    if fields.len() == 1 {
        return task_field_to_json(runtime, task, &fields[0]);
    }

    let mut object = serde_json::Map::new();
    for field in fields {
        object.insert(field.clone(), task_field_to_json(runtime, task, field)?);
    }
    Ok(Value::Object(object))
}

pub(super) fn print_task_fields(
    runtime: &OrbitRuntime,
    task: &orbit_core::Task,
    fields: &[String],
) -> Result<(), OrbitError> {
    if fields.len() == 1 {
        return print_single_task_field(runtime, task, &fields[0]);
    }

    use crate::output::color::bold;
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            println!();
        }
        println!("{} {}", bold("Field:"), field);
        print_single_task_field(runtime, task, field)?;
    }
    Ok(())
}

pub(super) fn print_single_task_field(
    runtime: &OrbitRuntime,
    task: &orbit_core::Task,
    field: &str,
) -> Result<(), OrbitError> {
    match field {
        "comments" => {
            use crate::output::color::dimmed;
            for comment in &task.comments {
                println!(
                    "{} {}: {}",
                    dimmed(&format!("[{}]", comment.at.to_rfc3339())),
                    comment.by,
                    comment.message
                );
            }
            Ok(())
        }
        "plan" => {
            print!("{}", task.plan);
            Ok(())
        }
        "execution_summary" => {
            print!("{}", task.execution_summary);
            Ok(())
        }
        "description" => {
            print!("{}", task.description);
            Ok(())
        }
        "acceptance_criteria" => {
            for criterion in &task.acceptance_criteria {
                println!("- {}", criterion);
            }
            Ok(())
        }
        "history" => {
            use crate::output::color::dimmed;
            for entry in &task.history {
                if let Some(note) = &entry.note {
                    println!(
                        "{} {}: {} ({})",
                        dimmed(&format!("[{}]", entry.at.to_rfc3339())),
                        entry.by,
                        entry.event,
                        note
                    );
                } else {
                    println!(
                        "{} {}: {}",
                        dimmed(&format!("[{}]", entry.at.to_rfc3339())),
                        entry.by,
                        entry.event
                    );
                }
            }
            Ok(())
        }
        "context_files" => {
            for path in &task.context_files {
                println!("{}", path);
            }
            Ok(())
        }
        "artifacts" => {
            use crate::output::color::bold;
            let artifacts = runtime.get_task_artifacts(&task.id)?;
            for (index, artifact) in artifacts.iter().enumerate() {
                if index > 0 {
                    println!();
                }
                println!("{} {}", bold("Artifact:"), artifact.path);
                print!("{}", artifact.content);
            }
            Ok(())
        }
        other => Err(OrbitError::InvalidInput(format!(
            "unknown field selector `{other}`. Valid values: comments, plan, execution_summary, description, acceptance_criteria, history, context_files, artifacts"
        ))),
    }
}

fn format_task_table_timestamp(value: DateTime<Utc>) -> String {
    value.format("%Y-%m-%d %H:%M").to_string()
}
