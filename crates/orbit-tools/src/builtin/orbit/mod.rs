pub mod activity_show;
pub mod duel_plan_add;
pub mod duel_plan_winner;
pub mod knowledge_add;
pub mod knowledge_delete;
pub mod knowledge_move;
pub mod knowledge_overview;
pub mod knowledge_pack;
pub mod knowledge_refs;
pub mod knowledge_search;
pub mod knowledge_show;
pub mod knowledge_write;
pub mod review_thread_add;
pub mod review_thread_list;
pub mod review_thread_reply;
pub mod review_thread_resolve;
pub mod state_get;
pub mod state_set;
pub mod task_add;
pub mod task_approve;
pub mod task_delete;
pub mod task_lint;
pub mod task_list;
pub mod task_locks;
pub mod task_reject;
pub mod task_show;
pub mod task_start;
pub mod task_update;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_knowledge::graph::nodes::CodebaseGraphV1;
use orbit_knowledge::graph::object_store::GraphObjectStore;
use orbit_store::state_io;
use orbit_types::{OrbitError, ToolParam, normalize_optional_attribution_label};
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
    registry.register(task_lint::OrbitTaskLintTool);
    registry.register(task_locks::OrbitTaskLocksTool);
    registry.register(task_start::OrbitTaskStartTool);
    registry.register(task_reject::OrbitTaskRejectTool);
    registry.register(task_show::OrbitTaskShowTool);
    registry.register(task_list::OrbitTaskListTool);
    registry.register(task_update::OrbitTaskUpdateTool);
    registry.register(duel_plan_add::OrbitDuelPlanAddTool);
    registry.register(duel_plan_winner::OrbitDuelPlanWinnerTool);
    registry.register(knowledge_add::OrbitKnowledgeAddTool);
    registry.register(knowledge_delete::OrbitKnowledgeDeleteTool);
    registry.register(knowledge_move::OrbitKnowledgeMoveTool);
    registry.register(knowledge_overview::OrbitKnowledgeOverviewTool);
    registry.register(knowledge_pack::OrbitKnowledgePackTool);
    registry.register(knowledge_refs::OrbitKnowledgeRefsTool);
    registry.register(knowledge_search::OrbitKnowledgeSearchTool);
    registry.register(knowledge_show::OrbitKnowledgeShowTool);
    registry.register(knowledge_write::OrbitKnowledgeWriteTool);
    registry.register(activity_show::OrbitActivityShowTool);
    registry.register(review_thread_add::OrbitReviewThreadAddTool);
    registry.register(review_thread_list::OrbitReviewThreadListTool);
    registry.register(review_thread_reply::OrbitReviewThreadReplyTool);
    registry.register(review_thread_resolve::OrbitReviewThreadResolveTool);
    registry.register(state_get::OrbitStateGetTool);
    registry.register(state_set::OrbitStateSetTool);
}

fn build_actor_label(agent: Option<&str>, model: Option<&str>) -> Option<String> {
    normalize_optional_attribution_label(model.or(agent), model)
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
            description: "Agent CLI family (codex, claude, or gemini).".to_string(),
            param_type: "string".to_string(),
            required: false,
        },
        ToolParam {
            name: "model".to_string(),
            description: "LLM model identifier (e.g. opus, gpt-5.4, gemini-3.1-pro-preview)."
                .to_string(),
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

pub(super) fn optional_raw_string(input: &Value, key: &str) -> Result<Option<String>, OrbitError> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let raw = value
                .as_str()
                .ok_or_else(|| OrbitError::InvalidInput(format!("`{key}` must be a string")))?;
            Ok(Some(raw.to_string()))
        }
    }
}

pub(super) fn has_explicit_knowledge_dir(input: &Value) -> bool {
    input
        .get("knowledge_dir")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty())
}

pub(super) fn maybe_refresh_knowledge_graph(
    ctx: &ToolContext,
    input: &Value,
    knowledge_dir: &Path,
) {
    if has_explicit_knowledge_dir(input) {
        return;
    }

    let Some(workspace_root) = ctx.workspace_root.as_deref() else {
        return;
    };

    let _ = orbit_knowledge::pipeline::ensure_fresh(knowledge_dir, workspace_root);
}

pub(super) fn load_graph_for_read(
    ctx: &ToolContext,
    input: &Value,
) -> Result<CodebaseGraphV1, OrbitError> {
    let knowledge_dir = knowledge_write::resolve_knowledge_dir(ctx, input)?;
    maybe_refresh_knowledge_graph(ctx, input, &knowledge_dir);

    GraphObjectStore::new(knowledge_dir.join("graph"))
        .read_graph()
        .map_err(|e| OrbitError::Execution(format!("failed to load knowledge graph: {e}")))
}

pub(super) fn resolve_state_dir(ctx: &ToolContext, input: &Value) -> Result<PathBuf, OrbitError> {
    if let Some(state_dir) = optional_string_alias(input, &["state_dir", "stateDir", "state-dir"])?
    {
        return Ok(PathBuf::from(state_dir));
    }
    if let Ok(state_dir) = std::env::var("ORBIT_STATE_DIR") {
        let trimmed = state_dir.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    let run_id = optional_string_alias(input, &["run_id", "runId", "run-id"])?.or_else(|| {
        std::env::var("ORBIT_RUN_ID")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    });
    let run_id = run_id.ok_or_else(|| {
        OrbitError::InvalidInput(
            "missing `state_dir`; provide `state_dir` or `run_id`, or set ORBIT_STATE_DIR/ORBIT_RUN_ID"
                .to_string(),
        )
    })?;

    let orbit_root = ctx
        .orbit_root
        .clone()
        .or_else(|| std::env::var("ORBIT_ROOT").ok().map(PathBuf::from))
        .ok_or_else(|| {
            OrbitError::InvalidInput(
                "cannot resolve active run path without orbit_root; pass `state_dir` explicitly"
                    .to_string(),
            )
        })?;

    state_io::resolve_active_run_state_dir(&orbit_root, &run_id)?
        .ok_or(OrbitError::JobRunNotFound(run_id))
}

pub(super) fn resolve_step_index(input: &Value) -> Result<u32, OrbitError> {
    if let Some(step_index) = optional_u32_alias(input, &["step_index", "stepIndex", "step-index"])?
    {
        return Ok(step_index);
    }
    let raw = std::env::var("ORBIT_STEP_INDEX").map_err(|_| {
        OrbitError::InvalidInput(
            "missing `step_index`; provide `step_index` or set ORBIT_STEP_INDEX".to_string(),
        )
    })?;
    raw.trim().parse::<u32>().map_err(|error| {
        OrbitError::InvalidInput(format!(
            "ORBIT_STEP_INDEX must be an unsigned integer: {error}"
        ))
    })
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

pub(super) fn optional_u32_alias(input: &Value, keys: &[&str]) -> Result<Option<u32>, OrbitError> {
    for key in keys {
        if let Some(value) = input.get(*key) {
            let raw = match value {
                Value::String(value) => value.trim().to_string(),
                Value::Number(value) => value.to_string(),
                _ => {
                    return Err(OrbitError::InvalidInput(format!(
                        "`{key}` must be a string or integer"
                    )));
                }
            };
            if raw.is_empty() {
                return Err(OrbitError::InvalidInput(format!(
                    "`{key}` must not be empty"
                )));
            }
            return raw.parse::<u32>().map(Some).map_err(|error| {
                OrbitError::InvalidInput(format!("`{key}` must be an unsigned integer: {error}"))
            });
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
