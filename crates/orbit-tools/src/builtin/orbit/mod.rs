pub mod adr;
pub mod docs;
pub mod duel;
pub mod friction;
pub mod graph;
pub mod graph_history;
pub mod groundhog;
pub mod knowledge;
pub mod learning;
pub mod pipeline;
pub mod review_thread;
pub mod search;
pub mod semantic;
pub mod state;
pub mod task;

use orbit_common::types::{
    OrbitError, ToolParam, normalize_agent_family_for_model, normalize_optional_attribution_label,
};
use serde::Serialize;
use serde_json::Value;

use crate::{
    GroundhogBuiltinAction, OrbitBuiltinAction, OrbitTaskScope, ToolContext, ToolRegistry,
};

pub(super) use orbit_common::types::{optional_string, optional_string_alias, required_string};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct OrbitIdentity {
    pub agent: Option<String>,
    pub model: Option<String>,
    pub actor_label: Option<String>,
}

pub fn register(registry: &mut ToolRegistry) {
    registry.register(adr::add::OrbitAdrAddTool);
    // ORB-00289: agents query ADR metadata via `orbit search --kind adr`;
    // `orbit.adr.list` stays available on the CLI / dashboard `runtime.run_tool`
    // path for admin workflows.
    registry.register_inactive(adr::list::OrbitAdrListTool);
    registry.register(adr::show::OrbitAdrShowTool);
    registry.register(adr::supersede::OrbitAdrSupersedeTool);
    registry.register(adr::update::OrbitAdrUpdateTool);
    registry.register_inactive(docs::OrbitDocsListTool);
    registry.register_inactive(docs::OrbitDocsShowTool);
    registry.register_inactive(docs::OrbitDocsAddTool);
    registry.register_inactive(docs::OrbitDocsIndexTool);
    registry.register_inactive(docs::OrbitDocsMigrateTool);
    registry.register(groundhog::checkpoint_success::OrbitGroundhogCheckpointSuccessTool);
    registry.register(groundhog::checkpoint_failure::OrbitGroundhogCheckpointFailureTool);
    registry.register(groundhog::side_effect::OrbitGroundhogSideEffectTool);
    registry.register(friction::add::OrbitFrictionAddTool);
    registry.register(friction::list::OrbitFrictionListTool);
    registry.register(friction::resolve::OrbitFrictionResolveTool);
    registry.register(friction::show::OrbitFrictionShowTool);
    registry.register_inactive(friction::stats::OrbitFrictionStatsTool);
    registry.register(friction::tags::OrbitFrictionTagsTool);
    registry.register(friction::update::OrbitFrictionUpdateTool);
    registry.register(task::add::OrbitTaskAddTool);
    registry.register(task::artifact_put::OrbitTaskArtifactPutTool);
    registry.register(task::approve::OrbitTaskApproveTool);
    // ORB-00289: destructive / admin-only — CLI subcommands still reach
    // them via `runtime.run_tool`; the agent MCP surface should not.
    registry.register_inactive(task::delete::OrbitTaskDeleteTool);
    registry.register_inactive(task::lint::OrbitTaskLintTool);
    registry.register_inactive(task::locks::OrbitTaskLocksTool);
    registry.register_inactive(task::locks_reserve::OrbitTaskLocksReserveTool);
    registry.register_inactive(task::locks_release::OrbitTaskLocksReleaseTool);
    registry.register(task::start::OrbitTaskStartTool);
    registry.register(task::reject::OrbitTaskRejectTool);
    registry.register(task::show::OrbitTaskShowTool);
    registry.register(task::list::OrbitTaskListTool);
    registry.register(task::update::OrbitTaskUpdateTool);
    registry.register(duel::plan_add::OrbitDuelPlanAddTool);
    registry.register(duel::plan_winner::OrbitDuelPlanWinnerTool);
    registry.register_inactive(graph_history::OrbitGraphHistoryTool);
    registry.register(graph::OrbitGraphSyncTool);
    registry.register(graph::OrbitGraphSearchTool);
    registry.register(graph::OrbitGraphShowTool);
    registry.register(graph::OrbitGraphRefsTool);
    registry.register(graph::OrbitGraphCalleesTool);
    registry.register(graph::OrbitGraphImpactTool);
    registry.register(graph::OrbitGraphTraceTool);
    registry.register(learning::add::OrbitLearningAddTool);
    registry.register(learning::comment_add::OrbitLearningCommentAddTool);
    // ORB-00289: destructive cleanup — admin-only, CLI path retains it.
    registry.register_inactive(learning::comment_delete::OrbitLearningCommentDeleteTool);
    registry.register(learning::comment_list::OrbitLearningCommentListTool);
    registry.register_inactive(learning::list::OrbitLearningListTool);
    // ORB-00289: destructive cleanup — admin-only, CLI path retains it.
    registry.register_inactive(learning::prune::OrbitLearningPruneTool);
    registry.register_inactive(learning::sync::OrbitLearningSyncTool);
    registry.register(learning::show::OrbitLearningShowTool);
    registry.register(learning::supersede::OrbitLearningSupersedeTool);
    registry.register(learning::update::OrbitLearningUpdateTool);
    registry.register(learning::upvote::OrbitLearningUpvoteTool);
    registry.register(pipeline::invoke::OrbitPipelineInvokeTool);
    registry.register(pipeline::wait::OrbitPipelineWaitTool);
    registry.register(review_thread::add::OrbitReviewThreadAddTool);
    registry.register(review_thread::add::OrbitReviewThreadAddAliasTool);
    registry.register(review_thread::list::OrbitReviewThreadListTool);
    registry.register(review_thread::list::OrbitReviewThreadListAliasTool);
    registry.register(review_thread::reply::OrbitReviewThreadReplyTool);
    registry.register(review_thread::reply::OrbitReviewThreadReplyAliasTool);
    registry.register(review_thread::resolve::OrbitReviewThreadResolveTool);
    registry.register(review_thread::resolve::OrbitReviewThreadResolveAliasTool);
    registry.register(search::OrbitSearchTool);
    registry.register_inactive(semantic::install::OrbitSemanticInstallTool);
    // ORB-00289: destructive teardown of the local semantic index —
    // admin-only, retained on the CLI surface (`orbit semantic uninstall`).
    registry.register_inactive(semantic::uninstall::OrbitSemanticUninstallTool);
    registry.register_inactive(semantic::stats::OrbitSemanticStatsTool);
    registry.register_inactive(semantic::index::OrbitSemanticIndexTool);
    registry.register(state::get::OrbitStateGetTool);
    registry.register(state::set::OrbitStateSetTool);
}

fn build_actor_label(agent: Option<&str>, model: Option<&str>) -> Option<String> {
    normalize_optional_attribution_label(model.or(agent), model)
}

fn trimmed_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(super) fn resolve_identity(
    ctx: &ToolContext,
    input: &Value,
) -> Result<OrbitIdentity, OrbitError> {
    let input_agent = optional_string_alias(input, &["agent"])?;
    let input_model = optional_string_alias(input, &["model"])?;
    let context_agent = trimmed_optional(ctx.agent_name.clone());
    let context_model = trimmed_optional(ctx.model_name.clone());
    let context_has_identity = context_agent.is_some() || context_model.is_some();
    let input_has_identity = input_agent
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || input_model
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
    let (agent, model) = if context_has_identity {
        let agent =
            normalize_agent_family_for_model(context_agent.as_deref(), context_model.as_deref())?;
        // Runtime-provided identity is authoritative at the tool boundary. If
        // an agent self-reports a `model` argument, Orbit overwrites it with
        // the canonical family string so downstream persistence compares
        // family identity, not unstable model aliases.
        let model = agent.clone();
        (agent, model)
    } else if input_has_identity {
        (trimmed_optional(input_agent), trimmed_optional(input_model))
    } else {
        (None, None)
    };
    let agent = normalize_agent_family_for_model(agent.as_deref(), model.as_deref())?;
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
            description:
                "Deprecated compatibility field. Prefer `model` with the agent family (`codex`, `claude`, `gemini`, or `grok`)."
                    .to_string(),
            param_type: "string".to_string(),
            required: false,
        },
        ToolParam {
            name: "model".to_string(),
            description:
                "Preferred provenance field. Pass the canonical agent family (`codex`, `claude`, `gemini`, or `grok`); full model strings are accepted and auto-normalized."
                    .to_string(),
            param_type: "string".to_string(),
            required: false,
        },
    ]
}

pub(super) fn model_identity_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "model".to_string(),
        description:
            "Preferred provenance field. Pass the canonical agent family (codex, claude, gemini, or grok); full model strings are accepted and auto-normalized."
                .to_string(),
        param_type: "string".to_string(),
        required: false,
    }]
}

pub(super) fn reject_agent_field(input: &Value, tool_name: &str) -> Result<(), OrbitError> {
    if input
        .as_object()
        .is_some_and(|object| object.contains_key("agent"))
    {
        return Err(OrbitError::InvalidInput(format!(
            "{tool_name} no longer accepts `agent`; use `model` with the agent family for attribution"
        )));
    }
    Ok(())
}

pub(super) fn scored_identity_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "agent".to_string(),
            description:
                "Deprecated compatibility field. Prefer `model` with the agent family (`codex`, `claude`, `gemini`, or `grok`)."
                    .to_string(),
            param_type: "string".to_string(),
            required: false,
        },
        ToolParam {
            name: "model".to_string(),
            description:
                "Required provenance field. Pass the canonical agent family (`codex`, `claude`, `gemini`, or `grok`), or `human` for human-authored review feedback to opt out of scoreboard scoring. Full model strings are accepted and auto-normalized."
                    .to_string(),
            param_type: "string".to_string(),
            required: true,
        },
    ]
}

pub(super) fn graph_ref_param() -> ToolParam {
    ToolParam {
        name: "ref".to_string(),
        description: "Ref.".to_string(),
        param_type: "string".to_string(),
        required: false,
    }
}

pub(super) fn execute_host_action(
    ctx: &ToolContext,
    input: Value,
    action: OrbitBuiltinAction,
) -> Result<Value, OrbitError> {
    let identity = resolve_identity(ctx, &input)?;
    require_orbit_host(ctx)?.execute(
        action,
        input,
        identity.agent,
        identity.model,
        ctx.reservation_owner.clone(),
    )
}

pub(super) fn resolve_workspace_argument(
    ctx: &ToolContext,
    input: &mut Value,
    tool_name: &str,
) -> Result<String, OrbitError> {
    // ADR-0181: MCP workspace defaults come from explicit session context, never process cwd.
    let explicit = optional_string_alias(input, &["workspace"])?;
    let session = ctx
        .session_context
        .workspace
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    match (explicit, session) {
        (Some(workspace), Some(session_workspace)) => {
            if workspace != session_workspace {
                tracing::info!(
                    target: "orbit.tools.workspace",
                    tool_name,
                    explicit_workspace = %workspace,
                    session_workspace = %session_workspace,
                    "explicit workspace overrides MCP session context"
                );
            }
            set_input_workspace(input, &workspace)?;
            Ok(workspace)
        }
        (Some(workspace), None) => {
            set_input_workspace(input, &workspace)?;
            Ok(workspace)
        }
        (None, Some(workspace)) => {
            set_input_workspace(input, &workspace)?;
            Ok(workspace)
        }
        (None, None) => Err(OrbitError::InvalidInput(
            "missing `workspace`; provide it explicitly or initialize the MCP session with `_meta.orbit.workspace`"
                .to_string(),
        )),
    }
}

fn set_input_workspace(input: &mut Value, workspace: &str) -> Result<(), OrbitError> {
    let Some(object) = input.as_object_mut() else {
        return Err(OrbitError::InvalidInput(
            "tool input must be a JSON object".to_string(),
        ));
    };
    object.insert(
        "workspace".to_string(),
        Value::String(workspace.to_string()),
    );
    Ok(())
}

pub(super) fn task_scope(ctx: &ToolContext) -> OrbitTaskScope {
    ctx.orbit_host
        .as_ref()
        .map(|host| host.task_scope())
        .unwrap_or_default()
}

fn require_orbit_host(ctx: &ToolContext) -> Result<&dyn crate::OrbitToolHost, OrbitError> {
    ctx.orbit_host.as_deref().ok_or_else(|| {
        OrbitError::Execution(
            "orbit builtin requires an Orbit runtime host in ToolContext".to_string(),
        )
    })
}

fn require_groundhog_host(ctx: &ToolContext) -> Result<&dyn crate::GroundhogToolHost, OrbitError> {
    ctx.groundhog_host.as_deref().ok_or_else(|| {
        OrbitError::Execution(
            "groundhog verb tools require an active groundhog runner context".to_string(),
        )
    })
}

pub(super) fn execute_groundhog_action<T: Serialize>(
    ctx: &ToolContext,
    action: GroundhogBuiltinAction,
    label: &str,
    input: &T,
) -> Result<Value, OrbitError> {
    let host = require_groundhog_host(ctx)?;
    let scope = host.scope();
    if !scope.active_day {
        return Err(OrbitError::Execution(format!(
            "groundhog {label} requires an active groundhog day context"
        )));
    }

    let input = serde_json::to_value(input)
        .map_err(|error| OrbitError::Execution(format!("groundhog {label} serialize: {error}")))?;
    host.execute(action, input)
}

pub(super) fn require_groundhog_fields(
    input: &Value,
    label: &str,
    fields: &[&str],
) -> Result<(), OrbitError> {
    let missing = input
        .as_object()
        .map(|obj| {
            fields
                .iter()
                .filter(|field| !obj.contains_key(**field))
                .copied()
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| fields.to_vec());

    if missing.is_empty() {
        return Ok(());
    }

    Err(OrbitError::InvalidInput(format!(
        "groundhog {label} input validation failed: missing required fields: {}",
        missing.join(", ")
    )))
}

/// Extract an optional string from the first matching key in `keys`.
///
/// Tools accept multiple key names for the same logical field to stay
/// friendly to agents that may use slightly different naming conventions
/// (e.g. `"type"`, `"task_type"`, `"taskType"` all map to the task type
/// parameter). The first non-absent key wins; absence of all keys returns
/// `None`. An explicitly empty value is rejected as an error.
pub(super) fn orbit_id_params(kind: &str) -> Vec<ToolParam> {
    vec![ToolParam {
        name: "id".to_string(),
        description: format!("{kind} ID"),
        param_type: "string".to_string(),
        required: true,
    }]
}

#[cfg(test)]
mod tests;
