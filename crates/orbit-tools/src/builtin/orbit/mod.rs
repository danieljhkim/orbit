pub mod adr;
pub mod auto_task;
pub mod docs;
pub mod duel;
pub mod friction;
pub mod learning;
pub mod operation;
pub mod pipeline;
pub mod search;
pub mod semantic;
pub mod state;
pub mod task;

use orbit_common::types::{
    McpToolPlacement, McpToolPolicy, OrbitError, ToolParam, normalize_agent_family_for_model,
    normalize_optional_attribution_label,
};
use serde_json::Value;

use crate::{OrbitBuiltinAction, OrbitTaskScope, ToolContext, ToolRegistry};

pub(super) use orbit_common::types::{optional_string_alias, required_string};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct OrbitIdentity {
    pub agent: Option<String>,
    pub model: Option<String>,
    pub actor_label: Option<String>,
}

pub fn register(registry: &mut ToolRegistry) {
    registry.register_mcp(
        adr::add::OrbitAdrAddTool,
        agent_operator(McpToolPlacement::Composite),
    );
    // ORB-00289: agents query ADR metadata via `orbit search --kind adr`;
    // `orbit.adr.list` stays available on the CLI / dashboard `runtime.run_tool`
    // path for admin workflows.
    registry.register_inactive(adr::list::OrbitAdrListTool);
    // Exact-id restore is an operator repair surface. Keep it available to
    // `orbit tool run` without advertising it to ordinary agent sessions.
    registry.register_inactive(adr::restore::OrbitAdrRestoreTool);
    registry.register_mcp(
        adr::show::OrbitAdrShowTool,
        agent_operator(McpToolPlacement::Owner),
    );
    registry.register_mcp(
        adr::supersede::OrbitAdrSupersedeTool,
        agent_operator(McpToolPlacement::Owner),
    );
    registry.register_mcp(
        adr::update::OrbitAdrUpdateTool,
        agent_operator(McpToolPlacement::Owner),
    );
    // Auto-task definitions [ORB-10149]: agents can define/retune/disable
    // recurring chores. `list` stays CLI/admin only (mirrors learning::list).
    registry.register_mcp(
        auto_task::add::OrbitAutoTaskAddTool,
        agent_operator(McpToolPlacement::Owner),
    );
    registry.register_inactive(auto_task::list::OrbitAutoTaskListTool);
    registry.register_mcp(
        auto_task::show::OrbitAutoTaskShowTool,
        agent_operator(McpToolPlacement::Owner),
    );
    registry.register_mcp(
        auto_task::update::OrbitAutoTaskUpdateTool,
        agent_operator(McpToolPlacement::Owner),
    );
    registry.register_mcp(
        auto_task::toggle::OrbitAutoTaskToggleTool,
        agent_operator(McpToolPlacement::Owner),
    );
    registry.register_inactive(docs::OrbitDocsListTool);
    registry.register_inactive(docs::OrbitDocsShowTool);
    registry.register_inactive(docs::OrbitDocsAddTool);
    registry.register_inactive(docs::OrbitDocsIndexTool);
    registry.register_inactive(docs::OrbitDocsMigrateTool);
    // ADR-0209 bearing 1 [ORB-10358]: every friction verb — its schema, its
    // placement, and whether MCP advertises it at all — is declared once in
    // `orbit_common::friction::operations` and registered from there. Adding a
    // friction verb needs no edit in this function.
    friction::register(registry);
    registry.register_mcp(
        task::add::OrbitTaskAddTool,
        agent_operator(McpToolPlacement::Hub),
    );
    registry.register_mcp(
        task::artifact_put::OrbitTaskArtifactPutTool,
        agent_operator(McpToolPlacement::Hub),
    );
    registry.register_mcp(
        task::approve::OrbitTaskApproveTool,
        agent_operator(McpToolPlacement::Hub),
    );
    // ORB-00289: destructive / admin-only — CLI subcommands still reach
    // them via `runtime.run_tool`; the agent MCP surface should not.
    registry.register_inactive(task::delete::OrbitTaskDeleteTool);
    registry.register_inactive(task::lint::OrbitTaskLintTool);
    registry.register_inactive(task::locks::OrbitTaskLocksTool);
    registry.register_inactive(task::locks_reserve::OrbitTaskLocksReserveTool);
    registry.register_inactive(task::locks_release::OrbitTaskLocksReleaseTool);
    registry.register_mcp(
        task::start::OrbitTaskStartTool,
        agent_operator(McpToolPlacement::Hub),
    );
    // Task rejection is a human/operator decision — CLI / dashboard only.
    registry.register_inactive(task::reject::OrbitTaskRejectTool);
    registry.register_mcp(
        task::show::OrbitTaskShowTool,
        agent_operator(McpToolPlacement::Hub),
    );
    registry.register_mcp(
        task::list::OrbitTaskListTool,
        agent_operator(McpToolPlacement::Hub),
    );
    registry.register_mcp(
        task::update::OrbitTaskUpdateTool,
        agent_operator(McpToolPlacement::Hub),
    );
    registry.register(duel::plan_add::OrbitDuelPlanAddTool);
    registry.register(duel::plan_winner::OrbitDuelPlanWinnerTool);
    registry.register_mcp(
        learning::add::OrbitLearningAddTool,
        agent_operator(McpToolPlacement::Composite),
    );
    registry.register_inactive(learning::list::OrbitLearningListTool);
    // ORB-00289: destructive cleanup — admin-only, CLI path retains it.
    registry.register_inactive(learning::prune::OrbitLearningPruneTool);
    registry.register_inactive(learning::sync::OrbitLearningSyncTool);
    registry.register_mcp(
        learning::show::OrbitLearningShowTool,
        agent_operator(McpToolPlacement::Owner),
    );
    registry.register_mcp(
        learning::supersede::OrbitLearningSupersedeTool,
        agent_operator(McpToolPlacement::Owner),
    );
    // ORB-10469: named single-learning retirement without a replacement,
    // gated the same as add/update/supersede (ADR-0250).
    registry.register_mcp(
        learning::archive::OrbitLearningArchiveTool,
        agent_operator(McpToolPlacement::Owner),
    );
    registry.register_mcp(
        learning::update::OrbitLearningUpdateTool,
        agent_operator(McpToolPlacement::Owner),
    );
    registry.register(pipeline::invoke::OrbitPipelineInvokeTool);
    registry.register(pipeline::wait::OrbitPipelineWaitTool);
    registry.register_mcp(
        search::OrbitSearchTool,
        agent_operator(McpToolPlacement::Composite),
    );
    registry.register_inactive(semantic::install::OrbitSemanticInstallTool);
    // ORB-00289: destructive teardown of the local semantic index —
    // admin-only, retained on the CLI surface (`orbit semantic uninstall`).
    registry.register_inactive(semantic::uninstall::OrbitSemanticUninstallTool);
    registry.register_inactive(semantic::stats::OrbitSemanticStatsTool);
    registry.register_inactive(semantic::index::OrbitSemanticIndexTool);
    registry.register(state::get::OrbitStateGetTool);
    registry.register(state::set::OrbitStateSetTool);
}

fn agent_operator(placement: McpToolPlacement) -> McpToolPolicy {
    McpToolPolicy::agent_and_operator(placement)
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
    // MCP workspace defaults come from explicit session context, never process cwd.
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
            "missing `workspace`; it must be a filesystem path to an existing directory inside \
             the repository (e.g. the repository root), never a logical workspace id such as a \
             bridge `ws_*` id — provide that path explicitly or initialize the MCP session with \
             `_meta.orbit.workspace` set to the same path"
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
