pub mod activity_show;
pub mod job_run_archive;
pub mod job_run_list;
pub mod job_run_show;
pub mod knowledge_pack;
pub mod knowledge_write;
pub mod review_thread_add;
pub mod review_thread_list;
pub mod review_thread_reply;
pub mod review_thread_resolve;
pub mod task_add;
pub mod task_approve;
pub mod task_delete;
pub mod task_list;
pub mod task_reject;
pub mod task_show;
pub mod task_start;
pub mod task_update;

use std::collections::HashMap;

use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{OrbitError, ToolParam};
use serde_json::Value;

use crate::{TIMEOUT_DEFAULT_MS, ToolContext, ToolRegistry};

const ORBIT_TASK_ACTOR_KIND: &str = "ORBIT_TASK_ACTOR_KIND";
const ORBIT_TASK_ACTOR_LABEL: &str = "ORBIT_TASK_ACTOR_LABEL";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct OrbitIdentity {
    pub agent: Option<String>,
    pub model: Option<String>,
    pub actor_label: Option<String>,
}

pub fn register(registry: &mut ToolRegistry) {
    registry.register(task_add::OrbitTaskAddTool);
    registry.register(task_approve::OrbitTaskApproveTool);
    registry.register(task_delete::OrbitTaskDeleteTool);
    registry.register(task_start::OrbitTaskStartTool);
    registry.register(task_reject::OrbitTaskRejectTool);
    registry.register(task_show::OrbitTaskShowTool);
    registry.register(task_list::OrbitTaskListTool);
    registry.register(task_update::OrbitTaskUpdateTool);
    registry.register(job_run_list::OrbitJobRunListTool);
    registry.register(job_run_show::OrbitJobRunShowTool);
    registry.register(job_run_archive::OrbitJobRunArchiveTool);
    registry.register(knowledge_pack::OrbitKnowledgePackTool);
    registry.register(knowledge_write::OrbitKnowledgeWriteTool);
    registry.register(activity_show::OrbitActivityShowTool);
    registry.register(review_thread_add::OrbitReviewThreadAddTool);
    registry.register(review_thread_list::OrbitReviewThreadListTool);
    registry.register(review_thread_reply::OrbitReviewThreadReplyTool);
    registry.register(review_thread_resolve::OrbitReviewThreadResolveTool);
}

fn build_actor_label(agent: Option<&str>, model: Option<&str>) -> Option<String> {
    match (agent, model) {
        (Some(agent), Some(model)) => Some(format!("{agent} / {model}")),
        (Some(agent), None) => Some(agent.to_string()),
        (None, Some(model)) => Some(model.to_string()),
        (None, None) => None,
    }
}

pub(super) fn resolve_identity(
    ctx: &ToolContext,
    input: &Value,
) -> Result<OrbitIdentity, OrbitError> {
    let agent = optional_string_alias(input, &["agent"])?.or_else(|| {
        ctx.agent_name
            .clone()
            .filter(|value| !value.trim().is_empty())
    });
    let model = optional_string_alias(input, &["model"])?.or_else(|| {
        ctx.model_name
            .clone()
            .filter(|value| !value.trim().is_empty())
    });
    let actor_label = build_actor_label(agent.as_deref(), model.as_deref());
    Ok(OrbitIdentity {
        agent,
        model,
        actor_label,
    })
}

pub(super) fn identity_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "agent".to_string(),
            description: "Optional agent name for precise Orbit provenance".to_string(),
            param_type: "string".to_string(),
            required: false,
        },
        ToolParam {
            name: "model".to_string(),
            description: "Optional agent model for precise Orbit provenance".to_string(),
            param_type: "string".to_string(),
            required: false,
        },
    ]
}

pub(super) fn append_identity_flags(args: &mut Vec<String>, identity: &OrbitIdentity) {
    if let Some(agent) = &identity.agent {
        args.push("--agent".to_string());
        args.push(agent.clone());
    }
    if let Some(model) = &identity.model {
        args.push("--model".to_string());
        args.push(model.clone());
    }
}

/// Build an [`ExecRequest`] that runs the `orbit` CLI with `args`.
///
/// The environment is deliberately rebuilt from the current process's env vars
/// rather than passed through wholesale, then `ORBIT_TASK_ACTOR_KIND=agent` is
/// injected. This lets the orbit CLI distinguish agent-initiated mutations from
/// human-initiated ones (e.g. for audit attribution and policy checks) without
/// requiring callers to set the variable themselves.
pub(super) fn orbit_exec_request_with_identity(
    ctx: &ToolContext,
    args: Vec<String>,
    identity: &OrbitIdentity,
) -> ExecRequest {
    let mut env = std::env::vars().collect::<HashMap<_, _>>();
    env.insert(ORBIT_TASK_ACTOR_KIND.to_string(), "agent".to_string());
    if let Some(actor_label) = &identity.actor_label {
        env.insert(ORBIT_TASK_ACTOR_LABEL.to_string(), actor_label.clone());
    }

    // Inject --root so the spawned orbit CLI resolves to the correct data root
    // regardless of the agent's working directory (e.g. inside a git worktree).
    let args = if let Some(root) = &ctx.orbit_root {
        let mut full = vec!["--root".to_string(), root.to_string_lossy().into_owned()];
        full.extend(args);
        full
    } else {
        args
    };

    ExecRequest {
        program: "orbit".to_string(),
        args,
        current_dir: ctx.cwd.clone(),
        timeout_ms: Some(TIMEOUT_DEFAULT_MS),
        stdin_mode: StdinMode::Null,
        environment_mode: EnvironmentMode::ClearAndSet(env.into_iter().collect()),
        debug: false,
    }
}

pub(super) fn run_orbit_json_command(req: ExecRequest, label: &str) -> Result<Value, OrbitError> {
    let result = run_process(&req, &NoSandbox)?;
    if !result.success {
        let stderr = result.stderr.trim();
        let detail = if stderr.is_empty() {
            "command returned non-zero exit status"
        } else {
            stderr
        };
        return Err(OrbitError::Execution(format!("{label} failed: {detail}")));
    }

    parse_json_output(label, &result.stdout)
}

pub(super) fn parse_json_output(label: &str, stdout: &str) -> Result<Value, OrbitError> {
    serde_json::from_str(stdout)
        .map_err(|e| OrbitError::Execution(format!("failed to parse {label} output: {e}")))
}

pub(super) fn required_string(
    input: &Value,
    keys: &[&str],
    canonical: &str,
) -> Result<String, OrbitError> {
    for key in keys {
        if let Some(value) = input.get(*key) {
            let raw = value
                .as_str()
                .ok_or_else(|| OrbitError::InvalidInput(format!("`{key}` must be a string")))?;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(OrbitError::InvalidInput(format!(
                    "`{key}` must not be empty"
                )));
            }
            return Ok(trimmed.to_string());
        }
    }

    Err(OrbitError::InvalidInput(format!("missing `{canonical}`")))
}

pub(super) fn optional_string(input: &Value, key: &str) -> Result<Option<String>, OrbitError> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let raw = value
                .as_str()
                .ok_or_else(|| OrbitError::InvalidInput(format!("`{key}` must be a string")))?;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(OrbitError::InvalidInput(format!(
                    "`{key}` must not be empty"
                )));
            }
            Ok(Some(trimmed.to_string()))
        }
    }
}

/// Extract an optional string from the first matching key in `keys`.
///
/// Tools accept multiple key names for the same logical field to stay
/// friendly to agents that may use slightly different naming conventions
/// (e.g. `"type"`, `"task_type"`, `"taskType"` all map to the task type
/// parameter). The first non-absent key wins; absence of all keys returns
/// `None`. An explicitly empty value is rejected as an error.
pub(super) fn optional_string_alias(
    input: &Value,
    keys: &[&str],
) -> Result<Option<String>, OrbitError> {
    for key in keys {
        if let Some(value) = input.get(*key) {
            let raw = value
                .as_str()
                .ok_or_else(|| OrbitError::InvalidInput(format!("`{key}` must be a string")))?;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(OrbitError::InvalidInput(format!(
                    "`{key}` must not be empty"
                )));
            }
            return Ok(Some(trimmed.to_string()));
        }
    }

    Ok(None)
}

pub(super) fn optional_string_list_alias(
    input: &Value,
    keys: &[&str],
) -> Result<Option<Vec<String>>, OrbitError> {
    for key in keys {
        if let Some(value) = input.get(*key) {
            return match value {
                Value::String(raw) => {
                    let trimmed = raw.trim();
                    if trimmed.is_empty() {
                        Err(OrbitError::InvalidInput(format!(
                            "`{key}` must not be empty"
                        )))
                    } else {
                        Ok(Some(vec![trimmed.to_string()]))
                    }
                }
                Value::Array(items) => {
                    let mut values = Vec::with_capacity(items.len());
                    for item in items {
                        let raw = item.as_str().ok_or_else(|| {
                            OrbitError::InvalidInput(format!("`{key}` entries must be strings"))
                        })?;
                        let trimmed = raw.trim();
                        if trimmed.is_empty() {
                            return Err(OrbitError::InvalidInput(format!(
                                "`{key}` entries must not be empty"
                            )));
                        }
                        values.push(trimmed.to_string());
                    }
                    Ok(Some(values))
                }
                _ => Err(OrbitError::InvalidInput(format!(
                    "`{key}` must be a string or array of strings"
                ))),
            };
        }
    }

    Ok(None)
}

pub(super) fn orbit_id_params(kind: &str) -> Vec<ToolParam> {
    vec![ToolParam {
        name: "id".to_string(),
        description: format!("{kind} ID"),
        param_type: "string".to_string(),
        required: true,
    }]
}
