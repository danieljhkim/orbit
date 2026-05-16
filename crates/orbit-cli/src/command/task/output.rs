use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use orbit_common::types::{ArtifactManifestFileV2, TaskArtifact};
use orbit_core::{
    OrbitError, OrbitRuntime, TaskStatus, build_task_status_index, resolve_task_dependencies,
};
use serde_json::{Value, json};

pub(crate) fn task_to_signal_json(task: &orbit_core::Task) -> Value {
    json!({
        "id": task.id,
        "parent_id": task.parent_id(),
        "title": task.title,
        "type": task.task_type.to_string(),
        "status": task.status.to_string(),
        "priority": task.priority.to_string(),
        "complexity": task.complexity.map(|value| value.to_string()),
    })
}

pub(crate) fn task_to_json(
    task: &orbit_core::Task,
    status_by_id: &BTreeMap<String, TaskStatus>,
) -> Value {
    json!({
        "id": task.id,
        "parent_id": task.parent_id(),
        "title": task.title,
        "description": task.description,
        "acceptance_criteria": task.acceptance_criteria,
        "dependencies": task.dependencies(),
        "resolved_dependencies": dependency_labels(task, status_by_id),
        "tags": task.tags,
        "plan": task.plan,
        "execution_summary": task.execution_summary,
        "context_files": task.context_files,
        "created_by": task.created_by,
        "planned_by": task.planned_by,
        "implemented_by": task.implemented_by,
        "status": task.status.to_string(),
        "priority": task.priority.to_string(),
        "complexity": task.complexity.map(|value| value.to_string()),
        "type": task.task_type.to_string(),
        "pr_status": task.pr_status,
        "external_refs": task.external_refs,
        "relations": task.relations,
        "source_task_id": task.source_task_id(),
        "job_run_id": task.job_run_id,
        "crew": task.crew,
        "created_at": task.created_at.to_rfc3339(),
        "updated_at": task.updated_at.to_rfc3339(),
    })
}

pub(crate) fn task_to_json_for_runtime(
    runtime: &OrbitRuntime,
    task: &orbit_core::Task,
) -> Result<Value, OrbitError> {
    let status_by_id = build_task_status_index(&runtime.list_tasks()?);
    task_to_json_with_sidecars(runtime, task, &status_by_id)
}

pub(crate) fn task_to_json_with_sidecars(
    runtime: &OrbitRuntime,
    task: &orbit_core::Task,
    status_by_id: &BTreeMap<String, TaskStatus>,
) -> Result<Value, OrbitError> {
    let mut value = task_to_json(task, status_by_id);
    let object = value.as_object_mut().ok_or_else(|| {
        OrbitError::Execution("task JSON projection did not produce an object".to_string())
    })?;
    object.insert(
        "comments".to_string(),
        serde_json::to_value(runtime.get_task_comments(&task.id)?)
            .map_err(|e| OrbitError::Io(e.to_string()))?,
    );
    object.insert(
        "history".to_string(),
        serde_json::to_value(runtime.get_task_history(&task.id)?)
            .map_err(|e| OrbitError::Io(e.to_string()))?,
    );
    object.insert(
        "review_threads".to_string(),
        serde_json::to_value(runtime.get_task_review_threads(&task.id)?)
            .map_err(|e| OrbitError::Io(e.to_string()))?,
    );
    object.insert(
        "artifacts".to_string(),
        task_artifact_manifest_to_json(&runtime.get_task_artifact_manifest(&task.id)?),
    );
    if let Some(projection) = runtime.resolved_crew_projection(task)? {
        object.insert("resolved_crew".to_string(), Value::String(projection.name));
        object.insert(
            "planner_model".to_string(),
            Value::String(projection.planner_model),
        );
        object.insert(
            "implementer_model".to_string(),
            Value::String(projection.implementer_model),
        );
        object.insert(
            "reviewer_model".to_string(),
            Value::String(projection.reviewer_model),
        );
    }
    Ok(value)
}

pub(super) fn task_lock_to_json(task: &orbit_core::Task) -> Value {
    json!({
        "id": task.id,
        "title": task.title,
        "status": task.status.to_string(),
        "job_run_id": task.job_run_id,
        "crew": task.crew,
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
            crate::output::color::task_type_color_cell(&task.task_type.to_string()),
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

        match task.job_run_id.as_deref() {
            Some(job_run_id) => println!(
                "[{}] {} ({}, job_run={})",
                task.id, task.title, task.status, job_run_id
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
    status_by_id: Option<&BTreeMap<String, TaskStatus>>,
) -> Result<Value, OrbitError> {
    match field {
        "comments" => serde_json::to_value(runtime.get_task_comments(&task.id)?)
            .map_err(|e| OrbitError::Io(e.to_string())),
        "plan" => Ok(Value::String(task.plan.clone())),
        "execution_summary" => Ok(Value::String(task.execution_summary.clone())),
        "description" => Ok(Value::String(task.description.clone())),
        "acceptance_criteria" => serde_json::to_value(&task.acceptance_criteria)
            .map_err(|e| OrbitError::Io(e.to_string())),
        "dependencies" => {
            serde_json::to_value(task.dependencies()).map_err(|e| OrbitError::Io(e.to_string()))
        }
        "tags" => serde_json::to_value(&task.tags).map_err(|e| OrbitError::Io(e.to_string())),
        "resolved_dependencies" => serde_json::to_value(dependency_labels(
            task,
            status_by_id.ok_or_else(|| {
                OrbitError::Execution(
                    "missing dependency status index for resolved_dependencies".to_string(),
                )
            })?,
        ))
        .map_err(|e| OrbitError::Io(e.to_string())),
        "history" => serde_json::to_value(runtime.get_task_history(&task.id)?)
            .map_err(|e| OrbitError::Io(e.to_string())),
        "context_files" => {
            serde_json::to_value(&task.context_files).map_err(|e| OrbitError::Io(e.to_string()))
        }
        "artifacts" => Ok(task_artifacts_to_json(
            &runtime.get_task_artifacts(&task.id)?,
        )),
        other => Err(OrbitError::InvalidInput(format!(
            "unknown field selector `{other}`. Valid values: comments, plan, execution_summary, description, acceptance_criteria, dependencies, resolved_dependencies, tags, history, context_files, artifacts"
        ))),
    }
}

pub(super) fn task_fields_to_json(
    runtime: &OrbitRuntime,
    task: &orbit_core::Task,
    fields: &[String],
    status_by_id: Option<&BTreeMap<String, TaskStatus>>,
) -> Result<Value, OrbitError> {
    if fields.len() == 1 {
        return task_field_to_json(runtime, task, &fields[0], status_by_id);
    }

    let mut object = serde_json::Map::new();
    for field in fields {
        object.insert(
            field.clone(),
            task_field_to_json(runtime, task, field, status_by_id)?,
        );
    }
    Ok(Value::Object(object))
}

pub(super) fn print_task_fields(
    runtime: &OrbitRuntime,
    task: &orbit_core::Task,
    fields: &[String],
    status_by_id: Option<&BTreeMap<String, TaskStatus>>,
) -> Result<(), OrbitError> {
    if fields.len() == 1 {
        return print_single_task_field(runtime, task, &fields[0], status_by_id);
    }

    use crate::output::color::bold;
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            println!();
        }
        println!("{} {}", bold("Field:"), field);
        print_single_task_field(runtime, task, field, status_by_id)?;
    }
    Ok(())
}

pub(super) fn print_single_task_field(
    runtime: &OrbitRuntime,
    task: &orbit_core::Task,
    field: &str,
    status_by_id: Option<&BTreeMap<String, TaskStatus>>,
) -> Result<(), OrbitError> {
    match field {
        "comments" => {
            use crate::output::color::dimmed;
            for comment in runtime.get_task_comments(&task.id)? {
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
        "dependencies" => {
            for dependency in task.dependencies() {
                println!("{}", dependency);
            }
            Ok(())
        }
        "tags" => {
            for tag in &task.tags {
                println!("{}", tag);
            }
            Ok(())
        }
        "resolved_dependencies" => {
            for dependency in dependency_labels(
                task,
                status_by_id.ok_or_else(|| {
                    OrbitError::Execution(
                        "missing dependency status index for resolved_dependencies".to_string(),
                    )
                })?,
            ) {
                println!("{}", dependency);
            }
            Ok(())
        }
        "history" => {
            use crate::output::color::dimmed;
            for entry in runtime.get_task_history(&task.id)? {
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
                println!(
                    "{} {} ({}, {} bytes)",
                    bold("Artifact:"),
                    artifact.path,
                    artifact.media_type,
                    artifact.content.len()
                );
                if let Some(content) = artifact.text_content() {
                    print!("{content}");
                } else {
                    println!("[binary content omitted]");
                }
            }
            Ok(())
        }
        other => Err(OrbitError::InvalidInput(format!(
            "unknown field selector `{other}`. Valid values: comments, plan, execution_summary, description, acceptance_criteria, dependencies, resolved_dependencies, tags, history, context_files, artifacts"
        ))),
    }
}

pub(crate) fn task_artifacts_to_json(artifacts: &[TaskArtifact]) -> Value {
    Value::Array(
        artifacts
            .iter()
            .map(|artifact| {
                let mut object = serde_json::Map::new();
                object.insert("path".to_string(), Value::String(artifact.path.clone()));
                object.insert(
                    "media_type".to_string(),
                    Value::String(artifact.media_type.clone()),
                );
                object.insert(
                    "size".to_string(),
                    Value::Number(serde_json::Number::from(artifact.content.len())),
                );
                if let Some(content) = artifact.text_content() {
                    object.insert("content".to_string(), Value::String(content.to_string()));
                }
                Value::Object(object)
            })
            .collect(),
    )
}

pub(crate) fn task_artifact_manifest_to_json(files: &[ArtifactManifestFileV2]) -> Value {
    Value::Array(
        files
            .iter()
            .map(|file| {
                json!({
                    "path": file.path,
                    "media_type": file.media_type,
                    "size_bytes": file.size_bytes,
                    "sha256": file.sha256,
                    "created_by": file.created_by,
                    "created_at": file.created_at.to_rfc3339(),
                })
            })
            .collect(),
    )
}

fn dependency_labels(
    task: &orbit_core::Task,
    status_by_id: &BTreeMap<String, TaskStatus>,
) -> Vec<String> {
    resolve_task_dependencies(task, status_by_id)
        .into_iter()
        .map(|dependency| dependency.label())
        .collect()
}

fn format_task_table_timestamp(value: DateTime<Utc>) -> String {
    value.format("%Y-%m-%d %H:%M").to_string()
}
