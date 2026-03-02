use std::str::FromStr;

use orbit_core::command::agent::AgentRunOptions;
use orbit_core::command::job::JobAddParams;
use orbit_core::command::scheduler::{SchedulerAddParams, SchedulerRunResult};
use orbit_core::command::skill::{SkillDoctorResult, SkillDoctorStatus};
use orbit_core::command::task::{TaskAddParams, TaskUpdateParams};
use orbit_core::command::tool::{DoctorResult, DoctorStatus};
use orbit_core::{
    Job, OrbitError, OrbitRuntime, Scheduler, SchedulerRetryBackoffStrategy, SchedulerRun,
    SchedulerTargetType, Task, TaskPriority, TaskStatus, TaskType,
};
use serde_json::{Map, Value, json};

use crate::result::{ToolCallEnvelope, ToolCallError};

struct IdentityContext {
    id: String,
    name: String,
    role: String,
    block: String,
}

pub fn dispatch_tool(runtime: &OrbitRuntime, tool: &str, args: Value) -> ToolCallEnvelope {
    let identity_id = extract_identity_id(&args);
    match dispatch_tool_inner(runtime, tool, &args) {
        Ok((identity, data)) => ToolCallEnvelope::success(
            tool,
            args,
            identity.id,
            identity.name,
            identity.role,
            identity.block,
            data,
        ),
        Err(err) => ToolCallEnvelope::failure(
            tool,
            args,
            identity_id,
            ToolCallError {
                code: orbit_error_code(&err).to_string(),
                message: err.to_string(),
            },
        ),
    }
}

fn dispatch_tool_inner(
    runtime: &OrbitRuntime,
    tool: &str,
    args: &Value,
) -> Result<(IdentityContext, Value), OrbitError> {
    let identity = resolve_identity_context(runtime, args)?;
    let obj = as_object(args)?;

    let data = match tool {
        "orbit.config.show" => config_show(runtime),
        "orbit.task.add" => task_add(runtime, obj, &identity),
        "orbit.task.list" => task_list(runtime, obj),
        "orbit.task.show" => task_show(runtime, obj),
        "orbit.task.update" => task_update(runtime, obj),
        "orbit.task.approve" => task_approve(runtime, obj, &identity),
        "orbit.task.close" => task_close(runtime, obj),
        "orbit.task.reopen" => task_reopen(runtime, obj),
        "orbit.task.delete" => task_delete(runtime, obj),
        "orbit.task.search" => task_search(runtime, obj),
        "orbit.job.add" => job_add(runtime, obj, &identity),
        "orbit.job.list" => job_list(runtime, obj),
        "orbit.job.show" => job_show(runtime, obj),
        "orbit.job.delete" => job_delete(runtime, obj),
        "orbit.scheduler.add" => scheduler_add(runtime, obj),
        "orbit.scheduler.list" => scheduler_list(runtime, obj),
        "orbit.scheduler.show" => scheduler_show(runtime, obj),
        "orbit.scheduler.run" => scheduler_run(runtime, obj),
        "orbit.scheduler.pause" => scheduler_pause(runtime, obj),
        "orbit.scheduler.resume" => scheduler_resume(runtime, obj),
        "orbit.scheduler.history" => scheduler_history(runtime, obj),
        "orbit.scheduler.delete" => scheduler_delete(runtime, obj),
        "orbit.agent.run" => agent_run(runtime, obj, &identity),
        "orbit.skill.list" => skill_list(runtime),
        "orbit.skill.show" => skill_show(runtime, obj),
        "orbit.skill.doctor" => skill_doctor(runtime),
        "orbit.tool.list" => tool_list(runtime),
        "orbit.tool.show" => tool_show(runtime, obj),
        "orbit.tool.run" => tool_run(runtime, obj),
        "orbit.tool.add" => tool_add(runtime, obj),
        "orbit.tool.remove" => tool_remove(runtime, obj),
        "orbit.tool.enable" => tool_enable(runtime, obj),
        "orbit.tool.disable" => tool_disable(runtime, obj),
        "orbit.tool.doctor" => tool_doctor(runtime),
        other => {
            return Err(OrbitError::InvalidInput(format!(
                "unknown MCP tool: {other}"
            )));
        }
    }?;

    Ok((identity, data))
}

fn config_show(runtime: &OrbitRuntime) -> Result<Value, OrbitError> {
    let orbit_root = runtime.data_root();
    let orbit_home = runtime.orbit_home();
    let config_path = runtime.config_path();
    let (inherit, pass) = runtime.execution_env_config();
    let persistence = runtime.persistence_config_json();
    let task_approval_required_for_agent = runtime.task_approval_required_for_agent();
    let task_delegate_approval = runtime.task_delegate_approval();
    let identity_root = runtime.identity_root();
    let identity_role_overrides = runtime.identity_role_overrides();

    Ok(json!({
        "root": orbit_root.to_string_lossy(),
        "home": orbit_home.to_string_lossy(),
        "path": config_path.to_string_lossy(),
        "exists": config_path.exists(),
        "execution": {
            "env": {
                "inherit": inherit,
                "pass": pass,
            }
        },
        "task": {
            "approval": {
                "required_for_agent": task_approval_required_for_agent,
                "delegate_approval": task_delegate_approval,
            }
        },
        "identity": {
            "root": identity_root.to_string_lossy(),
            "roles": identity_role_overrides,
        },
        "persistence": persistence,
    }))
}

fn task_add(
    runtime: &OrbitRuntime,
    obj: &Map<String, Value>,
    identity: &IdentityContext,
) -> Result<Value, OrbitError> {
    let title = required_string(obj, "title")?;
    let description = optional_string(obj, "description").unwrap_or_default();
    let instructions = optional_string(obj, "instructions").unwrap_or_default();
    let context_files = optional_string_array(obj, "context_files")?.unwrap_or_default();
    let workspace_path = optional_string(obj, "workspace_path");
    let task_identity =
        optional_string(obj, "task_identity_id").or_else(|| Some(identity.id.clone()));
    let assigned_to = optional_string(obj, "assigned_to");
    let created_by = optional_string(obj, "created_by");
    let priority = optional_string(obj, "priority")
        .map(|value| parse_enum::<TaskPriority>("priority", &value))
        .transpose()?
        .unwrap_or(TaskPriority::Medium);
    let task_type = optional_string(obj, "task_type")
        .map(|value| parse_enum::<TaskType>("task_type", &value))
        .transpose()?
        .unwrap_or(TaskType::Task);
    let owner = optional_string(obj, "owner").unwrap_or_default();
    let parent_id = optional_string(obj, "parent_id");

    let task = runtime.add_task(TaskAddParams {
        title,
        description,
        instructions,
        context_files,
        workspace_path,
        identity_id: task_identity,
        assigned_to,
        created_by,
        priority,
        task_type,
        owner,
        parent_id,
    })?;

    Ok(task_to_json(&task))
}

fn task_list(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let status = optional_string(obj, "status")
        .map(|value| parse_enum::<TaskStatus>("status", &value))
        .transpose()?;
    let priority = optional_string(obj, "priority")
        .map(|value| parse_enum::<TaskPriority>("priority", &value))
        .transpose()?;

    let tasks = if status.is_some() || priority.is_some() {
        runtime.list_tasks_filtered(status, priority)?
    } else {
        runtime.list_tasks()?
    };

    Ok(Value::Array(tasks.iter().map(task_to_json).collect()))
}

fn task_show(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let task_id = required_string(obj, "task_id")?;
    let task = runtime.get_task(&task_id)?;
    Ok(task_to_json(&task))
}

fn task_update(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let task_id = required_string(obj, "task_id")?;

    let workspace_path = optional_clearable_string(
        obj,
        "workspace_path",
        "clear_workspace_path",
        "workspace_path",
    )?;
    let identity_id = optional_clearable_string(
        obj,
        "task_identity_id",
        "clear_task_identity_id",
        "task_identity_id",
    )?;
    let assigned_to =
        optional_clearable_string(obj, "assigned_to", "clear_assigned_to", "assigned_to")?;
    let created_by =
        optional_clearable_string(obj, "created_by", "clear_created_by", "created_by")?;
    let parent_id = optional_clearable_string(obj, "parent_id", "clear_parent_id", "parent_id")?;

    let status = optional_string(obj, "status")
        .map(|value| parse_enum::<TaskStatus>("status", &value))
        .transpose()?;
    let priority = optional_string(obj, "priority")
        .map(|value| parse_enum::<TaskPriority>("priority", &value))
        .transpose()?;
    let task_type = optional_string(obj, "task_type")
        .map(|value| parse_enum::<TaskType>("task_type", &value))
        .transpose()?;

    let task = runtime.update_task(
        &task_id,
        TaskUpdateParams {
            title: optional_string(obj, "title"),
            description: optional_string(obj, "description"),
            instructions: optional_string(obj, "instructions"),
            context_files: optional_string_array(obj, "context_files")?,
            workspace_path,
            identity_id,
            assigned_to,
            created_by,
            status,
            priority,
            task_type,
            owner: optional_string(obj, "owner"),
            parent_id,
        },
    )?;

    Ok(task_to_json(&task))
}

fn task_approve(
    runtime: &OrbitRuntime,
    obj: &Map<String, Value>,
    identity: &IdentityContext,
) -> Result<Value, OrbitError> {
    let task_id = required_string(obj, "task_id")?;
    let approved_by = optional_string(obj, "approved_by").unwrap_or_else(|| identity.name.clone());
    let note = optional_string(obj, "note");

    let task = runtime.approve_task(&task_id, &approved_by, note)?;
    Ok(task_to_json(&task))
}

fn task_close(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let task_id = required_string(obj, "task_id")?;
    runtime.close_task(&task_id)?;
    let task = runtime.get_task(&task_id)?;
    Ok(task_to_json(&task))
}

fn task_reopen(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let task_id = required_string(obj, "task_id")?;
    runtime.reopen_task(&task_id)?;
    let task = runtime.get_task(&task_id)?;
    Ok(task_to_json(&task))
}

fn task_delete(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let task_id = required_string(obj, "task_id")?;
    runtime.delete_task(&task_id)?;
    Ok(json!({ "task_id": task_id, "deleted": true }))
}

fn task_search(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let query = required_string(obj, "query")?;
    let tasks = runtime.search_tasks(&query)?;
    Ok(Value::Array(tasks.iter().map(task_to_json).collect()))
}

fn job_add(
    runtime: &OrbitRuntime,
    obj: &Map<String, Value>,
    identity: &IdentityContext,
) -> Result<Value, OrbitError> {
    let job_id = required_string(obj, "job_id")?;
    let job_type = optional_string(obj, "job_type").unwrap_or_else(|| "general".to_string());
    let description = required_string(obj, "description")?;
    let input_schema_json =
        optional_object_value(obj, "input_schema_json")?.unwrap_or_else(|| json!({}));
    let output_schema_json =
        optional_object_value(obj, "output_schema_json")?.unwrap_or_else(|| json!({}));
    let artifact_path_template = optional_string(obj, "artifact_path_template");
    let skill_refs = optional_string_array(obj, "skill_refs")?.unwrap_or_default();
    let job_identity =
        optional_string(obj, "job_identity_id").or_else(|| Some(identity.id.clone()));

    let job = runtime.add_job(JobAddParams {
        id: job_id,
        spec_type: job_type,
        description,
        input_schema_json,
        output_schema_json,
        artifact_path_template,
        skill_refs,
        identity_id: job_identity,
        assigned_to: optional_string(obj, "assigned_to"),
        created_by: optional_string(obj, "created_by"),
    })?;

    Ok(job_to_json(&job))
}

fn job_list(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let include_inactive = optional_bool(obj, "include_inactive")?.unwrap_or(false);
    let jobs = runtime.list_jobs(include_inactive)?;
    Ok(Value::Array(jobs.iter().map(job_to_json).collect()))
}

fn job_show(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let job_id = required_string(obj, "job_id")?;
    let job = runtime.show_job(&job_id)?;
    Ok(job_to_json(&job))
}

fn job_delete(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let job_id = required_string(obj, "job_id")?;
    runtime.delete_job(&job_id)?;
    Ok(json!({ "job_id": job_id, "deleted": true }))
}

fn scheduler_add(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let target_id = required_string(obj, "target_id")?;
    let schedule = required_string(obj, "schedule")?;
    let agent_cli = required_string(obj, "agent_cli")?;
    let timeout = optional_string(obj, "timeout").unwrap_or_else(|| "5m".to_string());
    let retry_initial_delay =
        optional_string(obj, "retry_initial_delay").unwrap_or_else(|| "0s".to_string());

    let retry_max_attempts = optional_u32(obj, "retry_max_attempts")?.unwrap_or(0);
    let retry_backoff = optional_string(obj, "retry_backoff")
        .map(|value| parse_enum::<SchedulerRetryBackoffStrategy>("retry_backoff", &value))
        .transpose()?
        .unwrap_or(SchedulerRetryBackoffStrategy::None);

    let scheduler = runtime.add_scheduler(SchedulerAddParams {
        target_type: SchedulerTargetType::Job,
        target_id,
        schedule,
        agent_cli,
        timeout_seconds: parse_duration_seconds(&timeout)?,
        retry_max_attempts,
        retry_backoff_strategy: retry_backoff,
        retry_initial_delay_seconds: parse_duration_seconds(&retry_initial_delay)?,
    })?;

    Ok(scheduler_to_json(&scheduler))
}

fn scheduler_list(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let include_disabled = optional_bool(obj, "include_disabled")?.unwrap_or(false);
    let schedulers = runtime.list_schedulers(include_disabled)?;
    Ok(Value::Array(
        schedulers.iter().map(scheduler_to_json).collect(),
    ))
}

fn scheduler_show(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let scheduler_id = required_string(obj, "scheduler_id")?;
    let scheduler = runtime.show_scheduler(&scheduler_id)?;
    Ok(scheduler_to_json(&scheduler))
}

fn scheduler_run(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let scheduler_id = required_string(obj, "scheduler_id")?;
    let run = runtime.run_scheduler_now(&scheduler_id)?;
    scheduler_run_result_json(runtime, &scheduler_id, &run)
}

fn scheduler_pause(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let scheduler_id = required_string(obj, "scheduler_id")?;
    runtime.pause_scheduler(&scheduler_id)?;
    let scheduler = runtime.show_scheduler(&scheduler_id)?;
    Ok(scheduler_to_json(&scheduler))
}

fn scheduler_resume(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let scheduler_id = required_string(obj, "scheduler_id")?;
    runtime.resume_scheduler(&scheduler_id)?;
    let scheduler = runtime.show_scheduler(&scheduler_id)?;
    Ok(scheduler_to_json(&scheduler))
}

fn scheduler_history(
    runtime: &OrbitRuntime,
    obj: &Map<String, Value>,
) -> Result<Value, OrbitError> {
    let scheduler_id = required_string(obj, "scheduler_id")?;
    let runs = runtime.scheduler_history(&scheduler_id)?;
    Ok(Value::Array(
        runs.iter().map(scheduler_run_to_json).collect(),
    ))
}

fn scheduler_delete(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let scheduler_id = required_string(obj, "scheduler_id")?;
    runtime.delete_scheduler(&scheduler_id)?;
    Ok(json!({ "scheduler_id": scheduler_id, "deleted": true }))
}

fn agent_run(
    runtime: &OrbitRuntime,
    obj: &Map<String, Value>,
    identity: &IdentityContext,
) -> Result<Value, OrbitError> {
    let task_id = required_string(obj, "task_id")?;
    let result = runtime.run_agent_task_with_options(
        &task_id,
        AgentRunOptions {
            approve_on_verbal: optional_bool(obj, "approve_on_verbal")?.unwrap_or(false),
            approved_by: optional_string(obj, "approved_by"),
            approval_note: optional_string(obj, "approval_note"),
            identity_id: optional_string(obj, "run_identity_id")
                .or_else(|| Some(identity.id.clone())),
        },
    )?;

    Ok(json!({
        "session_id": result.session_id,
        "task_id": result.task_id,
        "tool_calls_executed": result.tool_calls_executed,
        "status": match result.status {
            orbit_core::AgentSessionStatus::Running => "running",
            orbit_core::AgentSessionStatus::Completed => "completed",
            orbit_core::AgentSessionStatus::Failed => "failed",
        },
    }))
}

fn skill_list(runtime: &OrbitRuntime) -> Result<Value, OrbitError> {
    let skills = runtime.list_file_skills()?;
    Ok(Value::Array(
        skills
            .iter()
            .map(|skill| {
                json!({
                    "id": skill.id,
                    "content_hash": skill.content_hash,
                    "path": skill.path,
                    "meta": skill.meta,
                })
            })
            .collect(),
    ))
}

fn skill_show(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let skill_id = required_string(obj, "skill_id")?;
    let skill = runtime.show_file_skill(&skill_id)?;
    Ok(json!({
        "id": skill.id,
        "path": skill.path,
        "content_hash": skill.content_hash,
        "content": skill.content,
        "sections": {
            "purpose": skill.sections.purpose,
            "behavioral_constraints": skill.sections.behavioral_constraints,
            "output_requirements": skill.sections.output_requirements,
            "evaluation_focus": skill.sections.evaluation_focus,
            "prohibitions": skill.sections.prohibitions,
            "examples": skill.sections.examples,
        },
        "meta": skill.meta,
        "meta_raw": skill.meta_raw,
        "output_schema": skill.output_schema,
    }))
}

fn skill_doctor(runtime: &OrbitRuntime) -> Result<Value, OrbitError> {
    let rows = runtime.doctor_file_skills()?;
    Ok(Value::Array(
        rows.iter().map(skill_doctor_to_json).collect(),
    ))
}

fn tool_list(runtime: &OrbitRuntime) -> Result<Value, OrbitError> {
    let tools = runtime.list_tools()?;
    Ok(Value::Array(
        tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "enabled": tool.enabled,
                    "builtin": tool.builtin,
                })
            })
            .collect(),
    ))
}

fn tool_show(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let tool_name = required_string(obj, "tool_name")?;
    let tool = runtime.show_tool(&tool_name)?;
    Ok(json!({
        "name": tool.name,
        "description": tool.description,
        "enabled": tool.enabled,
        "builtin": tool.builtin,
        "parameters": tool.parameters,
    }))
}

fn tool_run(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let tool_name = required_string(obj, "tool_name")?;
    let input = obj.get("input").cloned().unwrap_or_else(|| json!({}));
    let output = runtime.execute_tool_command(&tool_name, input)?;
    Ok(output)
}

fn tool_add(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let path = required_string(obj, "path")?;
    let tool_name = optional_string(obj, "name").unwrap_or_else(|| infer_tool_name(&path));
    let description = optional_string(obj, "description").unwrap_or_default();
    runtime.add_tool(&tool_name, &path, &description)?;
    Ok(json!({ "name": tool_name, "path": path }))
}

fn tool_remove(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let tool_name = required_string(obj, "tool_name")?;
    runtime.remove_tool(&tool_name)?;
    Ok(json!({ "tool_name": tool_name, "removed": true }))
}

fn tool_enable(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let tool_name = required_string(obj, "tool_name")?;
    runtime.enable_tool(&tool_name)?;
    let tool = runtime.show_tool(&tool_name)?;
    Ok(json!({
        "name": tool.name,
        "enabled": tool.enabled,
    }))
}

fn tool_disable(runtime: &OrbitRuntime, obj: &Map<String, Value>) -> Result<Value, OrbitError> {
    let tool_name = required_string(obj, "tool_name")?;
    runtime.disable_tool(&tool_name)?;
    let tool = runtime.show_tool(&tool_name)?;
    Ok(json!({
        "name": tool.name,
        "enabled": tool.enabled,
    }))
}

fn tool_doctor(runtime: &OrbitRuntime) -> Result<Value, OrbitError> {
    let rows = runtime.doctor()?;
    Ok(Value::Array(rows.iter().map(tool_doctor_to_json).collect()))
}

fn scheduler_run_result_json(
    runtime: &OrbitRuntime,
    scheduler_id: &str,
    run: &SchedulerRunResult,
) -> Result<Value, OrbitError> {
    let run_details = runtime
        .scheduler_history(scheduler_id)?
        .into_iter()
        .find(|entry| entry.run_id == run.run_id);

    Ok(json!({
        "scheduler_id": run.scheduler_id,
        "run_id": run.run_id,
        "state": run.state.to_string(),
        "attempt": run.attempt,
        "error_code": run_details.as_ref().and_then(|entry| entry.error_code.clone()),
        "error_message": run_details.as_ref().and_then(|entry| entry.error_message.clone()),
    }))
}

fn resolve_identity_context(
    runtime: &OrbitRuntime,
    args: &Value,
) -> Result<IdentityContext, OrbitError> {
    let obj = as_object(args)?;
    let identity_id = required_string(obj, "identity_id")?;
    let resolved = runtime.resolve_identity(&identity_id)?;
    let block = runtime.compile_identity_block(&resolved);
    Ok(IdentityContext {
        id: resolved.id,
        name: resolved.name,
        role: resolved.role.to_string(),
        block,
    })
}

fn extract_identity_id(args: &Value) -> Option<String> {
    args.as_object()
        .and_then(|obj| obj.get("identity_id"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn as_object(value: &Value) -> Result<&Map<String, Value>, OrbitError> {
    value
        .as_object()
        .ok_or_else(|| OrbitError::InvalidInput("tool arguments must be a JSON object".to_string()))
}

fn required_string(obj: &Map<String, Value>, key: &str) -> Result<String, OrbitError> {
    let Some(value) = obj.get(key) else {
        return Err(OrbitError::InvalidInput(format!(
            "missing required field '{key}'"
        )));
    };

    let Some(raw) = value.as_str() else {
        return Err(OrbitError::InvalidInput(format!(
            "'{key}' must be a string"
        )));
    };

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "'{key}' must not be empty"
        )));
    }

    Ok(trimmed.to_string())
}

fn optional_string(obj: &Map<String, Value>, key: &str) -> Option<String> {
    match obj.get(key) {
        Some(Value::String(raw)) => {
            let value = raw.trim();
            if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            }
        }
        _ => None,
    }
}

fn optional_bool(obj: &Map<String, Value>, key: &str) -> Result<Option<bool>, OrbitError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        _ => Err(OrbitError::InvalidInput(format!(
            "'{key}' must be a boolean"
        ))),
    }
}

fn optional_u32(obj: &Map<String, Value>, key: &str) -> Result<Option<u32>, OrbitError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .map(Some)
            .ok_or_else(|| OrbitError::InvalidInput(format!("'{key}' must be a non-negative u32"))),
        _ => Err(OrbitError::InvalidInput(format!(
            "'{key}' must be a number"
        ))),
    }
}

fn optional_string_array(
    obj: &Map<String, Value>,
    key: &str,
) -> Result<Option<Vec<String>>, OrbitError> {
    let Some(value) = obj.get(key) else {
        return Ok(None);
    };

    let Some(items) = value.as_array() else {
        return Err(OrbitError::InvalidInput(format!(
            "'{key}' must be an array of strings"
        )));
    };

    let mut values = Vec::new();
    for item in items {
        let Some(raw) = item.as_str() else {
            return Err(OrbitError::InvalidInput(format!(
                "'{key}' must contain strings"
            )));
        };
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            values.push(trimmed.to_string());
        }
    }

    Ok(Some(values))
}

fn optional_object_value(obj: &Map<String, Value>, key: &str) -> Result<Option<Value>, OrbitError> {
    let Some(value) = obj.get(key) else {
        return Ok(None);
    };

    if value.is_null() {
        return Ok(None);
    }
    if !value.is_object() {
        return Err(OrbitError::InvalidInput(format!(
            "'{key}' must be a JSON object"
        )));
    }
    Ok(Some(value.clone()))
}

fn optional_clearable_string(
    obj: &Map<String, Value>,
    field_key: &str,
    clear_key: &str,
    display_name: &str,
) -> Result<Option<Option<String>>, OrbitError> {
    let clear = optional_bool(obj, clear_key)?.unwrap_or(false);
    if clear {
        return Ok(Some(None));
    }

    if obj.contains_key(field_key) {
        return Ok(Some(optional_string(obj, field_key)));
    }

    if obj.contains_key(clear_key) {
        return Err(OrbitError::InvalidInput(format!(
            "'{clear_key}' can only be true/false for '{display_name}'"
        )));
    }

    Ok(None)
}

fn parse_duration_seconds(raw: &str) -> Result<u64, OrbitError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(OrbitError::InvalidInput(
            "duration must not be empty".to_string(),
        ));
    }

    let split_at = value
        .find(|ch: char| ch.is_alphabetic())
        .ok_or_else(|| OrbitError::InvalidInput(format!("invalid duration: {raw}")))?;
    let (num_raw, unit_raw) = value.split_at(split_at);

    let num: u64 = num_raw
        .parse()
        .map_err(|_| OrbitError::InvalidInput(format!("invalid duration number: {raw}")))?;

    let seconds = match unit_raw {
        "s" => num,
        "m" => num.saturating_mul(60),
        "h" => num.saturating_mul(3_600),
        "d" => num.saturating_mul(86_400),
        "w" => num.saturating_mul(604_800),
        _ => {
            return Err(OrbitError::InvalidInput(format!(
                "invalid duration unit: {unit_raw} (expected s/m/h/d/w)"
            )));
        }
    };

    Ok(seconds)
}

fn infer_tool_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string()
}

fn parse_enum<T>(field: &str, raw: &str) -> Result<T, OrbitError>
where
    T: FromStr,
    <T as FromStr>::Err: std::fmt::Display,
{
    raw.parse::<T>()
        .map_err(|e| OrbitError::InvalidInput(format!("invalid {field}: {e}")))
}

fn task_to_json(task: &Task) -> Value {
    json!({
        "id": task.id,
        "title": task.title,
        "description": task.description,
        "instructions": task.instructions,
        "context_files": task.context_files,
        "workspace_path": task.workspace_path,
        "identity_id": task.identity_id,
        "assigned_to": task.assigned_to,
        "created_by": task.created_by,
        "approved_at": task.approved_at.as_ref().map(|value| value.to_rfc3339()),
        "approved_by": task.approved_by,
        "approval_note": task.approval_note,
        "status": task.status.to_string(),
        "priority": task.priority.to_string(),
        "type": task.task_type.to_string(),
        "owner": task.owner,
        "parent_id": task.parent_id,
        "created_at": task.created_at.to_rfc3339(),
        "updated_at": task.updated_at.to_rfc3339(),
    })
}

fn job_to_json(job: &Job) -> Value {
    json!({
        "id": job.id,
        "type": job.spec_type,
        "description": job.description,
        "input_schema_json": job.input_schema_json,
        "output_schema_json": job.output_schema_json,
        "artifact_path_template": job.artifact_path_template,
        "skill_refs": job.skill_refs,
        "identity_id": job.identity_id,
        "assigned_to": job.assigned_to,
        "created_by": job.created_by,
        "is_active": job.is_active,
        "created_at": job.created_at.to_rfc3339(),
        "updated_at": job.updated_at.to_rfc3339(),
    })
}

fn scheduler_to_json(scheduler: &Scheduler) -> Value {
    json!({
        "scheduler_id": scheduler.scheduler_id,
        "target_type": scheduler.target_type.to_string(),
        "target_id": scheduler.target_id,
        "schedule": scheduler.schedule,
        "agent_cli": scheduler.agent_cli,
        "timeout_seconds": scheduler.timeout_seconds,
        "retry_max_attempts": scheduler.retry_max_attempts,
        "retry_backoff_strategy": scheduler.retry_backoff_strategy.to_string(),
        "retry_initial_delay_seconds": scheduler.retry_initial_delay_seconds,
        "state": scheduler.state.to_string(),
        "next_run_at": scheduler.next_run_at.to_rfc3339(),
        "created_at": scheduler.created_at.to_rfc3339(),
        "updated_at": scheduler.updated_at.to_rfc3339(),
    })
}

fn scheduler_run_to_json(run: &SchedulerRun) -> Value {
    json!({
        "run_id": run.run_id,
        "scheduler_id": run.scheduler_id,
        "attempt": run.attempt,
        "state": run.state.to_string(),
        "scheduled_at": run.scheduled_at.to_rfc3339(),
        "started_at": run.started_at.map(|value| value.to_rfc3339()),
        "finished_at": run.finished_at.map(|value| value.to_rfc3339()),
        "duration_ms": run.duration_ms,
        "exit_code": run.exit_code,
        "agent_response_json": run.agent_response_json,
        "error_code": run.error_code,
        "error_message": run.error_message,
        "created_at": run.created_at.to_rfc3339(),
    })
}

fn skill_doctor_to_json(row: &SkillDoctorResult) -> Value {
    json!({
        "skill_id": row.skill_name,
        "status": match row.status {
            SkillDoctorStatus::Ok => "ok",
            SkillDoctorStatus::Warning => "warning",
            SkillDoctorStatus::Error => "error",
        },
        "message": row.message,
    })
}

fn tool_doctor_to_json(row: &DoctorResult) -> Value {
    json!({
        "tool_name": row.tool_name,
        "status": match row.status {
            DoctorStatus::Ok => "ok",
            DoctorStatus::Warning => "warning",
            DoctorStatus::Error => "error",
        },
        "message": row.message,
    })
}

fn orbit_error_code(err: &OrbitError) -> &'static str {
    match err {
        OrbitError::PolicyDenied(_) => "POLICY_DENIED",
        OrbitError::ToolNotFound(_) => "TOOL_NOT_FOUND",
        OrbitError::TaskNotFound(_) => "TASK_NOT_FOUND",
        OrbitError::TaskApprovalRequired(_) => "TASK_APPROVAL_REQUIRED",
        OrbitError::SkillNotFound(_) => "SKILL_NOT_FOUND",
        OrbitError::SchedulerNotFound(_) => "JOB_NOT_FOUND",
        OrbitError::SchedulerRunNotFound(_) => "JOB_RUN_NOT_FOUND",
        OrbitError::JobNotFound(_) => "WORK_NOT_FOUND",
        OrbitError::AgentSessionNotFound(_) => "AGENT_SESSION_NOT_FOUND",
        OrbitError::IdentityNotFound(_) => "IDENTITY_NOT_FOUND",
        OrbitError::InvalidInput(_) => "INVALID_INPUT",
        OrbitError::SkillValidation(_) => "SKILL_VALIDATION_FAILED",
        OrbitError::IdentityValidation(_) => "IDENTITY_VALIDATION_FAILED",
        OrbitError::SchedulerValidation(_) => "JOB_VALIDATION_FAILED",
        OrbitError::AgentRun(_) => "AGENT_RUN_FAILED",
        OrbitError::AgentProtocolViolation(_) => "AGENT_PROTOCOL_VIOLATION",
        OrbitError::UnsupportedAgentProvider(_) => "UNSUPPORTED_AGENT_PROVIDER",
        OrbitError::Execution(_) => "EXECUTION_FAILED",
        OrbitError::Store(_) => "STORE_ERROR",
        OrbitError::Io(_) => "IO_ERROR",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::{Value, json};
    use tempfile::tempdir;

    use super::dispatch_tool;

    fn runtime_with_identity() -> (orbit_core::OrbitRuntime, tempfile::TempDir) {
        let dir = tempdir().expect("tempdir");
        let data_root = dir.path().join(".orbit");
        fs::create_dir_all(&data_root).expect("data root");

        let identity_root = dir.path().join("identities");
        fs::create_dir_all(&identity_root).expect("identity root");
        fs::write(
            identity_root.join("linus.yaml"),
            r#"identity:
  name: linus
  display_name: Linus
  role: leader
"#,
        )
        .expect("identity file");

        fs::write(
            data_root.join("config.toml"),
            format!(
                "[identity]\nroot = \"{}\"\n",
                identity_root.to_string_lossy().replace('\\', "\\\\")
            ),
        )
        .expect("config");

        (
            orbit_core::OrbitRuntime::from_data_root(&data_root).expect("runtime"),
            dir,
        )
    }

    #[test]
    fn requires_identity_id() {
        let (runtime, _dir) = runtime_with_identity();
        let response = dispatch_tool(&runtime, "orbit.task.list", json!({}));
        assert!(!response.ok);
        assert_eq!(response.error.expect("error").code, "INVALID_INPUT");
    }

    #[test]
    fn can_add_and_show_task() {
        let (runtime, _dir) = runtime_with_identity();
        let create = dispatch_tool(
            &runtime,
            "orbit.task.add",
            json!({
                "identity_id": "linus",
                "title": "mcp task",
                "task_type": "issue"
            }),
        );
        assert!(create.ok);
        let task_id = create
            .data
            .as_ref()
            .and_then(|data| data.get("id"))
            .and_then(Value::as_str)
            .expect("task id")
            .to_string();

        let show = dispatch_tool(
            &runtime,
            "orbit.task.show",
            json!({"identity_id": "linus", "task_id": task_id}),
        );
        assert!(show.ok);
    }
}
