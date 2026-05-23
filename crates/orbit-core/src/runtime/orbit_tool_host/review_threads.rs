use std::str::FromStr;

use orbit_common::types::{OrbitError, ReviewThreadStatus, optional_string, required_string};
use serde_json::Value;

use crate::OrbitRuntime;

use super::json::{serialize_error, serialize_task};

pub(super) fn add(
    runtime: &OrbitRuntime,
    input: Value,
    agent: Option<String>,
    model: Option<String>,
) -> Result<Value, OrbitError> {
    require_review_model(model.as_deref(), "orbit.task.review_thread.add")?;
    let id = review_thread_task_id(&input)?;
    let body = required_string(&input, &["body"], "body")?;
    let path = optional_string(&input, "path")?;
    let line = optional_string(&input, "line")?
        .map(|value| {
            value.parse::<u64>().map_err(|error| {
                OrbitError::InvalidInput(format!("`line` must be an unsigned integer: {error}"))
            })
        })
        .transpose()?;
    runtime.add_review_thread(&id, body, path, line, agent, model)?;
    serialize_task(runtime, &runtime.get_task(&id)?)
}

pub(super) fn list(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let id = review_thread_task_id(&input)?;
    let status = optional_string(&input, "status")?
        .map(|value| ReviewThreadStatus::from_str(&value))
        .transpose()
        .map_err(OrbitError::InvalidInput)?;
    serde_json::to_value(runtime.list_review_threads(&id, status)?)
        .map_err(serialize_error("serialize review threads"))
}

pub(super) fn reply(
    runtime: &OrbitRuntime,
    input: Value,
    agent: Option<String>,
    model: Option<String>,
) -> Result<Value, OrbitError> {
    require_review_model(model.as_deref(), "orbit.task.review_thread.reply")?;
    let id = review_thread_task_id(&input)?;
    let thread_id = required_string(&input, &["thread_id"], "thread_id")?;
    let body = required_string(&input, &["body"], "body")?;
    runtime.reply_review_thread(&id, &thread_id, body, agent, model)?;
    serialize_task(runtime, &runtime.get_task(&id)?)
}

pub(super) fn resolve(
    runtime: &OrbitRuntime,
    input: Value,
    agent: Option<String>,
    model: Option<String>,
) -> Result<Value, OrbitError> {
    let id = review_thread_task_id(&input)?;
    let thread_id = required_string(&input, &["thread_id"], "thread_id")?;
    runtime.resolve_review_thread(&id, &thread_id, agent, model)?;
    serialize_task(runtime, &runtime.get_task(&id)?)
}

fn require_review_model(model: Option<&str>, tool_name: &str) -> Result<(), OrbitError> {
    let resolved = model.map(str::trim).filter(|value| !value.is_empty());
    if resolved.is_none() {
        return Err(OrbitError::InvalidInput(format!(
            "{tool_name} requires `model`. Pass the calling agent family (`codex`, `claude`, `gemini`, or `grok`); pass `human` for human-authored review feedback to opt out of scoring."
        )));
    }
    Ok(())
}

fn review_thread_task_id(input: &Value) -> Result<String, OrbitError> {
    match required_string(input, &["id", "task_id"], "id") {
        Ok(task_id) => Ok(task_id),
        Err(OrbitError::InvalidInput(message)) if message == "missing `id`" => {
            crate::command::review_thread_hook::active_task_id_from_env().ok_or_else(|| {
                OrbitError::InvalidInput(format!(
                    "{message}; or set ORBIT_ACTIVE_TASK_ID for active-task review-thread operations"
                ))
            })
        }
        Err(error) => Err(error),
    }
}
