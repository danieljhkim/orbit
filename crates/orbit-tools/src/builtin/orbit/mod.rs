pub mod adr;
pub mod design;
pub mod docs;
pub mod duel;
pub mod friction;
pub mod graph_history;
pub mod groundhog;
pub mod knowledge;
pub mod learning;
pub mod pipeline;
pub mod review_thread;
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
    registry.register(adr::list::OrbitAdrListTool);
    registry.register(adr::show::OrbitAdrShowTool);
    registry.register(adr::supersede::OrbitAdrSupersedeTool);
    registry.register(adr::update::OrbitAdrUpdateTool);
    registry.register(design::init::OrbitDesignInitTool);
    registry.register(design::list::OrbitDesignListTool);
    registry.register(design::show::OrbitDesignShowTool);
    registry.register(docs::OrbitDocsListTool);
    registry.register(docs::OrbitDocsShowTool);
    registry.register(docs::OrbitDocsSearchTool);
    registry.register(docs::OrbitDocsAddTool);
    registry.register(docs::OrbitDocsReindexTool);
    registry.register(docs::OrbitDocsMigrateTool);
    registry.register(groundhog::checkpoint_success::OrbitGroundhogCheckpointSuccessTool);
    registry.register(groundhog::checkpoint_failure::OrbitGroundhogCheckpointFailureTool);
    registry.register(groundhog::side_effect::OrbitGroundhogSideEffectTool);
    registry.register(friction::add::OrbitFrictionAddTool);
    registry.register(friction::list::OrbitFrictionListTool);
    registry.register(friction::resolve::OrbitFrictionResolveTool);
    registry.register(friction::show::OrbitFrictionShowTool);
    registry.register(friction::stats::OrbitFrictionStatsTool);
    registry.register(friction::tags::OrbitFrictionTagsTool);
    registry.register(friction::update::OrbitFrictionUpdateTool);
    registry.register(task::add::OrbitTaskAddTool);
    registry.register(task::artifact_put::OrbitTaskArtifactPutTool);
    registry.register(task::approve::OrbitTaskApproveTool);
    registry.register(task::delete::OrbitTaskDeleteTool);
    registry.register(task::lint::OrbitTaskLintTool);
    registry.register(task::locks::OrbitTaskLocksTool);
    registry.register(task::locks_reserve::OrbitTaskLocksReserveTool);
    registry.register(task::locks_release::OrbitTaskLocksReleaseTool);
    registry.register(task::start::OrbitTaskStartTool);
    registry.register(task::reject::OrbitTaskRejectTool);
    registry.register(task::show::OrbitTaskShowTool);
    registry.register(task::list::OrbitTaskListTool);
    registry.register(task::search::OrbitTaskSearchTool);
    registry.register(task::update::OrbitTaskUpdateTool);
    registry.register(duel::plan_add::OrbitDuelPlanAddTool);
    registry.register(duel::plan_winner::OrbitDuelPlanWinnerTool);
    registry.register(graph_history::OrbitGraphHistoryTool);
    registry.register(knowledge::callers::OrbitKnowledgeCallersTool);
    registry.register(knowledge::deps::OrbitKnowledgeDepsTool);
    registry.register(knowledge::implementors::OrbitKnowledgeImplementorsTool);
    registry.register(knowledge::overview::OrbitKnowledgeOverviewTool);
    registry.register(knowledge::pack::OrbitKnowledgePackTool);
    registry.register(knowledge::refs::OrbitKnowledgeRefsTool);
    registry.register(knowledge::search::OrbitKnowledgeSearchTool);
    registry.register(knowledge::show::OrbitKnowledgeShowTool);
    registry.register(learning::add::OrbitLearningAddTool);
    registry.register(learning::comment_add::OrbitLearningCommentAddTool);
    registry.register(learning::comment_delete::OrbitLearningCommentDeleteTool);
    registry.register(learning::comment_list::OrbitLearningCommentListTool);
    registry.register(learning::list::OrbitLearningListTool);
    registry.register(learning::prune::OrbitLearningPruneTool);
    registry.register(learning::reindex::OrbitLearningReindexTool);
    registry.register(learning::search::OrbitLearningSearchTool);
    registry.register(learning::show::OrbitLearningShowTool);
    registry.register(learning::supersede::OrbitLearningSupersedeTool);
    registry.register(learning::update::OrbitLearningUpdateTool);
    registry.register(learning::upvote::OrbitLearningUpvoteTool);
    registry.register(pipeline::invoke::OrbitPipelineInvokeTool);
    registry.register(pipeline::wait::OrbitPipelineWaitTool);
    registry.register(review_thread::add::OrbitReviewThreadAddTool);
    registry.register(review_thread::list::OrbitReviewThreadListTool);
    registry.register(review_thread::reply::OrbitReviewThreadReplyTool);
    registry.register(review_thread::resolve::OrbitReviewThreadResolveTool);
    registry.register(semantic::search::OrbitSemanticSearchTool);
    registry.register(semantic::related::OrbitSemanticRelatedTool);
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
            "Preferred provenance field. Pass the canonical agent family (`codex`, `claude`, `gemini`, or `grok`); full model strings are accepted and auto-normalized."
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
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn runtime_identity_overwrites_self_reported_model_at_tool_boundary() {
        let ctx = tool_context("claude", "claude-opus-4-7");

        let identity =
            resolve_identity(&ctx, &json!({ "model": "opus-4.7" })).expect("identity resolves");

        assert_eq!(identity.agent.as_deref(), Some("claude"));
        assert_eq!(identity.model.as_deref(), Some("claude"));
        assert_eq!(identity.actor_label.as_deref(), Some("claude"));
    }

    fn tool_context(agent: &str, model: &str) -> ToolContext {
        ToolContext {
            cwd: None,
            allowed_tools: Vec::new(),
            workspace_root: None,
            agent_name: Some(agent.to_string()),
            model_name: Some(model.to_string()),
            role_slot: None,
            proc_allowed_programs: Vec::new(),
            policy_engine: None,
            fs_profile: None,
            fs_audit: None,
            reservation_owner: None,
            orbit_host: None,
            groundhog_host: None,
        }
    }
}
