use orbit_common::types::{AgentFamily, OrbitError, RoleSlot, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitDuelPlanAddTool;

fn expected_signature(family: AgentFamily, slot: RoleSlot) -> String {
    format!("*authored by: {} / {}*", family.as_str(), slot.as_str())
}

fn role_slot(ctx: &ToolContext, input: &Value) -> Result<RoleSlot, OrbitError> {
    if let Some(slot) = ctx.role_slot {
        return Ok(slot);
    }
    input
        .get("planning_duel_slot")
        .or_else(|| input.get("role_slot"))
        .or_else(|| input.get("slot"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            OrbitError::InvalidInput(
                "orbit.duel.plan.add requires planning_duel_slot to derive the artifact path"
                    .to_string(),
            )
        })?
        .parse()
}

fn build_update_input(ctx: &ToolContext, input: &Value) -> Result<Value, OrbitError> {
    let identity = super::super::resolve_identity(ctx, input)?;
    let family = identity.agent.as_deref().ok_or_else(|| {
        OrbitError::InvalidInput(
            "orbit.duel.plan.add requires agent identity to derive the artifact path".to_string(),
        )
    })?;
    let family = AgentFamily::parse(family)?;
    let slot = role_slot(ctx, input)?;
    if slot.planner_slot().is_none() {
        return Err(OrbitError::InvalidInput(
            "orbit.duel.plan.add only accepts planner_a or planner_b slots".to_string(),
        ));
    }
    let id = super::super::required_string(input, &["id"], "id")?;
    let content = super::super::required_string(input, &["content", "plan"], "content")?;
    let first_line = content.lines().next().map(str::trim).unwrap_or_default();
    let expected = expected_signature(family, slot);
    let content = if first_line == expected {
        content
    } else {
        format!("{expected}\n{}", content.trim_start())
    };

    Ok(json!({
        "id": id,
        "artifacts": [{
            "path": format!("planning-duel/{}.md", slot.as_str()),
            "content": content,
        }],
        "agent": family.as_str(),
        "model": family.as_str(),
    }))
}

impl Tool for OrbitDuelPlanAddTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::super::orbit_id_params("task");
        parameters.push(ToolParam {
            name: "content".to_string(),
            description: "Planner markdown body to persist. Orbit stamps `*authored by: <family> / <slot>*` as the first line when absent.".to_string(),
            param_type: "string".to_string(),
            required: true,
        });
        parameters.push(ToolParam {
            name: "planning_duel_slot".to_string(),
            description: "Planning-duel slot for this proposal: planner_a or planner_b."
                .to_string(),
            param_type: "string".to_string(),
            required: false,
        });
        parameters.extend(super::super::identity_params());
        ToolSchema {
            name: "orbit.duel.plan.add".to_string(),
            description: "Persist one planning-duel proposal under `planning-duel/<slot>.md` for the calling family and slot.".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(
            ctx,
            build_update_input(ctx, &input)?,
            OrbitBuiltinAction::TaskUpdate,
        )
    }
}
