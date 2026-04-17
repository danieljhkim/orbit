pub mod activity_show;
pub mod duel_plan_add;
pub mod duel_plan_winner;
pub mod knowledge_add;
pub mod knowledge_callers;
pub mod knowledge_delete;
pub mod knowledge_deps;
pub mod knowledge_implementors;
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

use orbit_knowledge::TaskGraphService;
use orbit_knowledge::graph::nodes::CodebaseGraphV1;
use orbit_types::{OrbitError, ToolParam, normalize_optional_attribution_label};
use serde_json::Value;

use crate::{OrbitBuiltinAction, OrbitTaskScope, ToolContext, ToolRegistry};

pub(super) use orbit_types::{optional_string, optional_string_alias, required_string};

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
    registry.register(knowledge_callers::OrbitKnowledgeCallersTool);
    registry.register(knowledge_delete::OrbitKnowledgeDeleteTool);
    registry.register(knowledge_deps::OrbitKnowledgeDepsTool);
    registry.register(knowledge_implementors::OrbitKnowledgeImplementorsTool);
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

pub(super) fn execute_host_action(
    ctx: &ToolContext,
    input: Value,
    action: OrbitBuiltinAction,
) -> Result<Value, OrbitError> {
    let identity = resolve_identity(ctx, &input)?;
    require_orbit_host(ctx)?.execute(action, input, identity.agent, identity.model)
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

pub(super) fn has_explicit_knowledge_dir(input: &Value) -> bool {
    input
        .get("knowledge_dir")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty())
}

pub(super) fn load_graph_for_read(
    ctx: &ToolContext,
    input: &Value,
) -> Result<CodebaseGraphV1, OrbitError> {
    let knowledge_dir = knowledge_write::resolve_knowledge_dir(ctx, input)?;
    let service = TaskGraphService::new(knowledge_dir, knowledge_write::task_graph_scope(ctx));
    service.read_graph(
        ctx.workspace_root.as_deref(),
        has_explicit_knowledge_dir(input),
    )
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
