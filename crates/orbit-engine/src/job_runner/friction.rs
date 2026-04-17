use chrono::{DateTime, Utc};
use orbit_store::friction_log::append_friction_entry;
use orbit_store::metrics_log::append_metrics_entry;
use orbit_types::{ActorIdentity, FrictionEntry, MetricsEntry};
use serde_json::Value;
use std::path::Path;

use crate::context::{EnvironmentHost, ExecutorLookupHost, RuntimeHost};

use super::helpers::{extract_task_id, normalize_agent_label, resolved_model_name};

#[derive(Default)]
pub(super) struct FrictionContext {
    pub(super) input: Option<Value>,
    pub(super) command: Option<String>,
    pub(super) agent: Option<String>,
    pub(super) model: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn append_failed_step_friction<H: RuntimeHost + EnvironmentHost + ExecutorLookupHost>(
    data_root: &Path,
    host: &H,
    run_id: &str,
    step_id: &str,
    execution: &crate::context::ExecutionContext,
    exit_code: Option<i32>,
    stderr: &str,
    ts: DateTime<Utc>,
) {
    append_failed_step_friction_without_execution(
        data_root,
        run_id,
        step_id,
        FrictionContext {
            input: Some(execution.input.clone()),
            command: Some(command_label(execution)),
            agent: (!execution.agent_cli.trim().is_empty())
                .then(|| normalize_agent_label(&execution.agent_cli)),
            model: resolved_model_name(host, execution),
        },
        exit_code,
        stderr,
        ts,
    );
}

pub(super) fn append_failed_step_friction_without_execution(
    data_root: &Path,
    run_id: &str,
    step_id: &str,
    context: FrictionContext,
    exit_code: Option<i32>,
    stderr: &str,
    ts: DateTime<Utc>,
) {
    let input = context
        .input
        .unwrap_or_else(|| Value::Object(Default::default()));
    let actor_identity =
        ActorIdentity::from_legacy(context.agent.as_deref(), context.model.as_deref());
    let entry = FrictionEntry {
        ts,
        job_run: run_id.to_string(),
        step: step_id.to_string(),
        task_id: extract_task_id(&input).map(ToOwned::to_owned),
        command: context.command.unwrap_or_else(|| step_id.to_string()),
        input: serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string()),
        exit_code,
        stderr: stderr.to_string(),
        actor_identity,
    };
    if let Err(error) = append_friction_entry(data_root, &entry) {
        eprintln!("orbit: failed to append friction log entry: {error}");
    }
}

fn command_label(execution: &crate::context::ExecutionContext) -> String {
    let config = &execution.activity.spec_config;
    match execution.activity.spec_type.as_str() {
        "automation" => config
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or(execution.activity.id.as_str())
            .to_string(),
        "cli_command" => config
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or(execution.activity.id.as_str())
            .to_string(),
        "agent_invoke" => normalize_agent_label(&execution.agent_cli),
        _ => execution.activity.id.to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn append_step_metrics<H: RuntimeHost + EnvironmentHost + ExecutorLookupHost>(
    data_root: &Path,
    host: &H,
    run_id: &str,
    step_id: &str,
    execution: &crate::context::ExecutionContext,
    duration_ms: Option<u64>,
    tool_invocations: u32,
    token_usage: Option<u64>,
    retry_count: u32,
    ts: DateTime<Utc>,
) {
    let agent = (!execution.agent_cli.trim().is_empty())
        .then(|| normalize_agent_label(&execution.agent_cli));
    let model = resolved_model_name(host, execution);
    let task_id = extract_task_id(&execution.input).map(ToOwned::to_owned);

    let actor_identity = ActorIdentity::from_legacy(agent.as_deref(), model.as_deref());
    let entry = MetricsEntry {
        ts,
        job_run: run_id.to_string(),
        step: step_id.to_string(),
        task_id,
        actor_identity,
        tool_invocations,
        token_usage,
        step_duration_ms: duration_ms,
        retry_count,
    };
    if let Err(error) = append_metrics_entry(data_root, &entry) {
        eprintln!("orbit: failed to append metrics log entry: {error}");
    }
}
