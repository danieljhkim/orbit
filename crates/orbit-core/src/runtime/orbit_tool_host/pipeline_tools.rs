use orbit_common::types::{
    OrbitError, normalize_optional_attribution_label, optional_string, required_string,
};
use serde_json::Value;

use crate::OrbitRuntime;

use super::input::{
    parse_optional_poll_interval_seconds, parse_optional_timeout_seconds, parse_string_array_field,
    parse_task_priority, require_object_field,
};
use super::json::serialize_error;

pub(super) fn invoke(
    runtime: &OrbitRuntime,
    input: Value,
    agent: Option<String>,
    model: Option<String>,
) -> Result<Value, OrbitError> {
    let job_name = required_string(&input, &["job_name"], "job_name")?;
    let payload = require_object_field(&input, "input")?.clone();
    let priority = optional_string(&input, "priority")?
        .map(|value| parse_task_priority("priority", &value))
        .transpose()?
        .map(|value| value.to_string());
    let actor = Some(
        normalize_optional_attribution_label(
            model.as_deref().or(agent.as_deref()),
            model.as_deref(),
        )
        .unwrap_or_else(|| runtime.actor_label().to_string()),
    )
    .filter(|value| !value.trim().is_empty());
    serde_json::to_value(runtime.submit_pipeline_run(
        &job_name,
        payload,
        priority.as_deref(),
        actor.as_deref(),
    )?)
    .map_err(serialize_error("serialize pipeline invoke"))
}

pub(super) fn wait(
    runtime: &OrbitRuntime,
    input: Value,
    agent: Option<String>,
    model: Option<String>,
) -> Result<Value, OrbitError> {
    let run_ids = parse_string_array_field(&input, "run_ids")?;
    let timeout_seconds =
        OrbitRuntime::normalize_pipeline_wait_timeout(parse_optional_timeout_seconds(&input)?)?;
    let poll_interval_seconds = OrbitRuntime::normalize_pipeline_wait_poll_interval(
        parse_optional_poll_interval_seconds(&input)?,
    );
    let actor = Some(
        normalize_optional_attribution_label(
            model.as_deref().or(agent.as_deref()),
            model.as_deref(),
        )
        .unwrap_or_else(|| runtime.actor_label().to_string()),
    )
    .filter(|value| !value.trim().is_empty());
    serde_json::to_value(runtime.wait_pipeline_runs(
        &run_ids,
        timeout_seconds,
        poll_interval_seconds,
        actor.as_deref(),
    )?)
    .map_err(serialize_error("serialize pipeline wait"))
}
