//! Minimal duplication of the `*_to_json` projection helpers that the dashboard
//! API delegates to. These were originally in orbit-cli under command/* but are
//! duplicated here (verbatim logic) so orbit-dashboard compiles in isolation
//! without a dependency on orbit-cli (per ARCHITECTURE layering rules).

use std::collections::{BTreeMap, BTreeSet};

use orbit_common::types::{ArtifactManifestFileV2, JobV2Step, JobV2StepBody, PipelineState};
use orbit_core::command::job::JobCatalogEntry;
use orbit_core::{
    AuditEvent, EvidenceKind, JobRun, Learning, OrbitError, OrbitRuntime, Task, TaskStatus,
    resolve_task_dependencies,
};
use serde_json::{Value, json};

pub(crate) fn audit_event_to_json(event: &AuditEvent) -> Value {
    json!({
        "id": event.id,
        "execution_id": event.execution_id,
        "timestamp": event.timestamp.to_rfc3339(),
        "command": event.command,
        "subcommand": event.subcommand,
        "tool_name": event.tool_name,
        "target_type": event.target_type,
        "target_id": event.target_id,
        "role": event.role,
        "status": event.status.to_string(),
        "exit_code": event.exit_code,
        "duration_ms": event.duration_ms,
        "working_directory": event.working_directory,
        "arguments_json": event.arguments_json,
        "stdout_truncated": event.stdout_truncated,
        "stderr_truncated": event.stderr_truncated,
        "error_message": event.error_message,
        "host": event.host,
        "pid": event.pid,
        "session_id": event.session_id,
        "task_id": event.task_id,
        "job_run_id": event.job_run_id,
        "activity_id": event.activity_id,
        "step_index": event.step_index,
    })
}

pub(crate) fn job_catalog_to_json_with_last_run(
    job: &JobCatalogEntry,
    last_run: Option<&JobRun>,
) -> Value {
    let mut value = json!({
        "job_id": job.job_id.clone(),
        "kind": job.kind().to_string(),
        "state": job.state().to_string(),
        "default_input": job.spec.default_input,
        "max_active_runs": job.spec.max_active_runs,
        "steps": job.spec.steps.iter().map(job_v2_step_to_json).collect::<Vec<_>>(),
        "path": job.path.display().to_string(),
    });
    value["last_run_state"] = last_run
        .map(|r| serde_json::Value::String(r.state.to_string()))
        .unwrap_or(serde_json::Value::Null);
    value["last_run_at"] = last_run
        .and_then(|r| r.finished_at.or(r.started_at).or(Some(r.scheduled_at)))
        .map(|ts| serde_json::Value::String(ts.to_rfc3339()))
        .unwrap_or(serde_json::Value::Null);
    value
}

fn job_v2_step_to_json(step: &JobV2Step) -> Value {
    let mut value = json!({
        "id": step.id.clone(),
        "when": step.when,
        "retry": step.retry,
    });
    match &step.body {
        JobV2StepBody::TargetRef(target) => {
            value["body"] = json!({
                "kind": "target_ref",
                "target": target.target.clone(),
                "default_input": target.default_input,
                "timeout_seconds": target.timeout_seconds,
                "session": target.session,
            });
        }
        JobV2StepBody::Target(target) => {
            value["body"] = json!({
                "kind": "target",
                "default_input": target.default_input,
                "timeout_seconds": target.timeout_seconds,
                "session": target.session,
                "spec": target.spec,
            });
        }
        JobV2StepBody::Parallel { parallel } => {
            value["body"] = json!({
                "kind": "parallel",
                "join": parallel.join,
                "branches": parallel.branches.iter().map(job_v2_step_to_json).collect::<Vec<_>>(),
            });
        }
        JobV2StepBody::FanOut { fan_out, fan_in } => {
            value["body"] = json!({
                "kind": "fan_out",
                "items": fan_out.items,
                "max_workers": fan_out.max_workers,
                "worker": job_v2_step_to_json(&fan_out.worker),
                "fan_in": fan_in,
            });
        }
        JobV2StepBody::Loop { loop_ } => {
            value["body"] = json!({
                "kind": "loop",
                "max_iterations": loop_.max_iterations,
                "break_when": loop_.break_when,
                "steps": loop_.steps.iter().map(job_v2_step_to_json).collect::<Vec<_>>(),
            });
        }
    }
    value
}

pub(crate) fn job_run_to_json(run: &JobRun) -> Value {
    job_run_to_json_with_state(run, None)
}

pub(crate) fn job_run_to_json_with_state(run: &JobRun, state: Option<&PipelineState>) -> Value {
    let last = run.steps.last();
    let state = (!run.state.is_terminal()).then_some(state).flatten();
    let waiting_on_deps = state
        .and_then(|state| state.waiting_on_deps.as_ref())
        .filter(|values| !values.is_empty());
    let waiting_on_locks = state
        .and_then(|state| state.waiting_on_locks.as_ref())
        .filter(|values| !values.is_empty());
    json!({
        "run_id": run.run_id,
        "job_id": run.job_id,
        "attempt": run.attempt,
        "state": run.state.to_string(),
        "waiting_on_deps": waiting_on_deps,
        "waiting_on_locks": waiting_on_locks,
        "scheduled_at": run.scheduled_at.to_rfc3339(),
        "started_at": run.started_at.map(|v| v.to_rfc3339()),
        "finished_at": run.finished_at.map(|v| v.to_rfc3339()),
        "duration_ms": run.duration_ms,
        "retry_source_run_id": run.retry_source_run_id,
        "exit_code": last.and_then(|s| s.exit_code),
        "agent_response_json": last.and_then(|s| s.agent_response_json.as_ref()),
        "error_code": last.and_then(|s| s.error_code.as_deref()),
        "error_message": last.and_then(|s| s.error_message.as_deref()),
        "knowledge_metrics": run.knowledge_metrics,
        "resolved_crew": run.resolved_crew,
        "planner_model": run.planner_model,
        "implementer_model": run.implementer_model,
        "reviewer_model": run.reviewer_model,
        "steps": run.steps.iter().map(|s| json!({
            "step_index": s.step_index,
            "target_type": s.target_type.to_string(),
            "target_id": s.target_id,
            "state": s.state.to_string(),
            "started_at": s.started_at.map(|v| v.to_rfc3339()),
            "finished_at": s.finished_at.map(|v| v.to_rfc3339()),
            "duration_ms": s.duration_ms,
            "exit_code": s.exit_code,
            "agent_response_json": s.agent_response_json,
            "error_code": s.error_code,
            "error_message": s.error_message,
        })).collect::<Vec<_>>(),
        "created_at": run.created_at.to_rfc3339(),
    })
}

pub(crate) fn task_to_json(task: &Task, status_by_id: &BTreeMap<String, TaskStatus>) -> Value {
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

pub(crate) fn task_to_json_with_sidecars(
    runtime: &OrbitRuntime,
    task: &Task,
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

fn dependency_labels(task: &Task, status_by_id: &BTreeMap<String, TaskStatus>) -> Vec<String> {
    resolve_task_dependencies(task, status_by_id)
        .into_iter()
        .map(|dependency| dependency.label())
        .collect()
}

pub(crate) fn task_lock_to_json(task: &Task) -> Value {
    json!({
        "id": task.id,
        "title": task.title,
        "status": task.status.to_string(),
        "job_run_id": task.job_run_id,
        "crew": task.crew,
        "context_files": task.context_files,
    })
}

pub(crate) fn task_locks_json(runtime: &OrbitRuntime) -> Result<Value, OrbitError> {
    let (tasks, locked_files) = task_locks(runtime)?;
    let json_by_task: Vec<Value> = tasks.iter().map(task_lock_to_json).collect();
    Ok(json!({
        "locked_files": locked_files.iter().cloned().collect::<Vec<_>>(),
        "by_task": json_by_task,
        "total_locked": locked_files.len(),
        "total_tasks": tasks.len(),
    }))
}

fn task_locks(runtime: &OrbitRuntime) -> Result<(Vec<Task>, BTreeSet<String>), OrbitError> {
    let mut tasks: Vec<_> = runtime
        .list_tasks()?
        .into_iter()
        .filter(|task| matches!(task.status, TaskStatus::InProgress | TaskStatus::Review))
        .collect();

    tasks.sort_by_key(|task| {
        (
            task_lock_status_rank(task.status),
            task.created_at,
            task.id.clone(),
        )
    });

    let locked_files: BTreeSet<String> = tasks
        .iter()
        .flat_map(|task| task.context_files.iter().cloned())
        .collect();

    Ok((tasks, locked_files))
}

fn task_lock_status_rank(status: TaskStatus) -> u8 {
    match status {
        TaskStatus::InProgress => 0,
        TaskStatus::Review => 1,
        _ => 2,
    }
}

pub(crate) fn learning_to_json(learning: &Learning) -> Value {
    json!({
        "id": learning.id,
        "status": learning.status.as_str(),
        "scope": {
            "paths": learning.scope.paths,
            "tags": learning.scope.tags,
            "symbols": learning.scope.symbols,
            "semantic_seed": learning.scope.semantic_seed,
        },
        "summary": learning.summary,
        "body": learning.body,
        "evidence": learning
            .evidence
            .iter()
            .map(|e| json!({"kind": evidence_kind_str(e.kind), "ref": e.reference}))
            .collect::<Vec<_>>(),
        "supersedes": learning.supersedes,
        "superseded_by": learning.superseded_by,
        "created_at": learning.created_at.to_rfc3339(),
        "updated_at": learning.updated_at.to_rfc3339(),
        "created_by": learning.created_by,
        "priority": learning.priority,
    })
}

fn evidence_kind_str(kind: EvidenceKind) -> &'static str {
    match kind {
        EvidenceKind::Task => "task",
        EvidenceKind::Commit => "commit",
        EvidenceKind::External => "external",
    }
}
