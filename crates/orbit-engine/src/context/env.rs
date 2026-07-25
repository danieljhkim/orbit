//! Environment-variable resolution applied on top of an [`EnvironmentMode`]:
//! explicit `env_set` overrides and the `ORBIT_*` run-state vars.

use orbit_exec::EnvironmentMode;
use serde_json::Value;

use super::execution::ExecutionContext;

#[derive(Debug, Default)]
pub(crate) struct ProvenanceEnv<'a> {
    pub(crate) orbit_run_id: Option<&'a str>,
    pub(crate) orbit_managed_run_context: bool,
    pub(crate) orbit_agent_name: Option<&'a str>,
    pub(crate) orbit_agent_model: Option<&'a str>,
    pub(crate) orbit_session_id: Option<&'a str>,
    pub(crate) orbit_task_id: Option<&'a str>,
    pub(crate) orbit_active_task: bool,
    pub(crate) agent_run_id: Option<&'a str>,
    pub(crate) agent_model: Option<&'a str>,
    pub(crate) agent_task_id: Option<&'a str>,
}

/// Builds subprocess provenance variables shared by every engine spawn path.
///
/// The namespaces deliberately remain separate: `ORBIT_*` is internal
/// plumbing whose consumers depend on Orbit job-run semantics, while
/// `AGENT_*` is a spawner-neutral, cross-repository commit-telemetry contract
/// whose model value must be the exact provider model string. Callers select
/// only the fields their spawn site historically emitted.
pub(crate) fn provenance_env(config: ProvenanceEnv<'_>) -> Vec<(String, String)> {
    let mut vars = Vec::new();

    if let Some(run_id) = config.orbit_run_id {
        vars.push(("ORBIT_RUN_ID".to_string(), run_id.to_string()));
    }
    if config.orbit_managed_run_context {
        vars.push(("ORBIT_MANAGED_RUN_CONTEXT".to_string(), "1".to_string()));
    }
    if let Some(agent_name) = config.orbit_agent_name {
        vars.push(("ORBIT_AGENT_NAME".to_string(), agent_name.to_string()));
    }
    if let Some(model) = config.orbit_agent_model {
        vars.push(("ORBIT_AGENT_MODEL".to_string(), model.to_string()));
    }
    if let Some(session_id) = config.orbit_session_id {
        vars.push(("ORBIT_SESSION_ID".to_string(), session_id.to_string()));
    }
    if let Some(task_id) = config.orbit_task_id {
        vars.push(("ORBIT_TASK_ID".to_string(), task_id.to_string()));
        if config.orbit_active_task {
            vars.push(("ORBIT_ACTIVE_TASK_ID".to_string(), task_id.to_string()));
        }
    }
    if let Some(run_id) = config.agent_run_id {
        vars.push(("AGENT_RUN_ID".to_string(), run_id.to_string()));
    }
    if let Some(model) = config.agent_model {
        vars.push(("AGENT_MODEL".to_string(), model.to_string()));
    }
    if let Some(task_id) = config.agent_task_id {
        vars.push(("AGENT_TASK".to_string(), task_id.to_string()));
    }

    vars
}

/// Resolve `${VAR}` references in a value string from the parent environment.
/// Returns an empty string and logs a warning when the referenced var is not set.
/// Previously the literal `${VAR}` was passed through, which caused tools like `gh`
/// to receive an invalid token value.
fn resolve_env_refs(value: &str) -> String {
    if let Some(inner) = value.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
        match std::env::var(inner) {
            Ok(resolved) => resolved,
            Err(_) => {
                tracing::warn!(
                    target: "orbit.engine.env",
                    var = inner,
                    "env_set references an environment variable that is not set; substituting empty string",
                );
                String::new()
            }
        }
    } else {
        value.to_string()
    }
}

/// Apply explicit key-value env vars (`env_set`) on top of an already-resolved
/// [`EnvironmentMode`].  Values may contain `${VAR}` references that are
/// resolved from the parent environment.  Entries in `env_set` override
/// same-named vars.
pub fn apply_env_set(
    mode: EnvironmentMode,
    env_set: &std::collections::HashMap<String, String>,
) -> EnvironmentMode {
    if env_set.is_empty() {
        return mode;
    }
    let apply = |pairs: &mut Vec<(String, String)>| {
        for (key, raw_value) in env_set {
            let value = resolve_env_refs(raw_value);
            if let Some(existing) = pairs.iter_mut().find(|(k, _)| k == key) {
                existing.1 = value;
            } else {
                pairs.push((key.clone(), value));
            }
        }
    };
    match mode {
        EnvironmentMode::ClearAndSet(mut pairs) => {
            apply(&mut pairs);
            EnvironmentMode::ClearAndSet(pairs)
        }
        EnvironmentMode::Inherit => {
            let mut pairs: Vec<(String, String)> = std::env::vars().collect();
            apply(&mut pairs);
            EnvironmentMode::ClearAndSet(pairs)
        }
    }
}

pub fn state_env_vars(execution: &ExecutionContext) -> Vec<(String, String)> {
    let mut vars: Vec<(String, String)> = Vec::new();

    // Always export the activity identifier when we have one — it survives
    // even when the run/state vars are absent (e.g. ad-hoc activity invocation
    // outside a job run). Audit consumers use this to attribute tool calls.
    if !execution.activity.id.is_empty() {
        vars.push((
            "ORBIT_ACTIVITY_ID".to_string(),
            execution.activity.id.clone(),
        ));
    }

    // Task ID is sourced from the activity input by convention (see
    // `execution_working_directory_with_task` for the same pattern).
    let task_id = execution
        .input
        .get("task_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    // ADR-0182: hooks read the explicit active-task binding while older
    // audit/tool code continues to consume ORBIT_TASK_ID.
    vars.extend(provenance_env(ProvenanceEnv {
        orbit_task_id: task_id,
        orbit_active_task: true,
        ..ProvenanceEnv::default()
    }));

    // Run-state vars only exist for steps inside a real job run, so they
    // share a guarded block.
    if let (Some(run_id), Some(step_index), Some(state_dir)) = (
        execution.run_id.as_ref(),
        execution.step_index,
        execution.state_dir.as_ref(),
    ) {
        vars.extend(provenance_env(ProvenanceEnv {
            orbit_run_id: Some(run_id),
            orbit_managed_run_context: true,
            ..ProvenanceEnv::default()
        }));
        vars.push(("ORBIT_STEP_INDEX".to_string(), step_index.to_string()));
        vars.push((
            "ORBIT_STATE_DIR".to_string(),
            state_dir.to_string_lossy().into_owned(),
        ));
    }

    vars
}

pub fn inject_state_env(mode: EnvironmentMode, execution: &ExecutionContext) -> EnvironmentMode {
    let state_env = state_env_vars(execution);
    if state_env.is_empty() {
        return mode;
    }
    let apply = |pairs: &mut Vec<(String, String)>| {
        for (key, value) in &state_env {
            if let Some(existing) = pairs
                .iter_mut()
                .find(|(existing_key, _)| existing_key == key)
            {
                existing.1 = value.clone();
            } else {
                pairs.push((key.clone(), value.clone()));
            }
        }
    };
    match mode {
        EnvironmentMode::ClearAndSet(mut pairs) => {
            apply(&mut pairs);
            EnvironmentMode::ClearAndSet(pairs)
        }
        EnvironmentMode::Inherit => {
            let mut pairs: Vec<(String, String)> = std::env::vars().collect();
            apply(&mut pairs);
            EnvironmentMode::ClearAndSet(pairs)
        }
    }
}
