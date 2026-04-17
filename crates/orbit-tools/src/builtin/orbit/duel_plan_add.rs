use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitDuelPlanAddTool;

fn expected_signature(agent: &str, model: &str) -> String {
    format!("*authored by: {agent} / {model}*")
}

fn build_update_input(ctx: &ToolContext, input: &Value) -> Result<Value, OrbitError> {
    let identity = super::resolve_identity(ctx, input)?;
    let agent = identity.agent.clone().ok_or_else(|| {
        OrbitError::InvalidInput(
            "orbit.duel.plan.add requires agent identity to derive the artifact path".to_string(),
        )
    })?;
    let model = identity.model.clone().ok_or_else(|| {
        OrbitError::InvalidInput(
            "orbit.duel.plan.add requires model identity to derive the artifact path".to_string(),
        )
    })?;
    let id = super::required_string(input, &["id"], "id")?;
    let content = super::required_string(input, &["content", "plan"], "content")?;
    let first_line = content.lines().next().map(str::trim).unwrap_or_default();
    let expected = expected_signature(&agent, &model);
    if first_line != expected {
        return Err(OrbitError::InvalidInput(format!(
            "planner artifact content must start with `{expected}`"
        )));
    }

    Ok(json!({
        "id": id,
        "artifacts": [{
            "path": format!("planning-duel/{agent}-{model}.md"),
            "content": content,
        }],
        "agent": agent,
        "model": model,
    }))
}

impl Tool for OrbitDuelPlanAddTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::orbit_id_params("task");
        parameters.push(ToolParam {
            name: "content".to_string(),
            description: "Planner markdown to persist. The first line must match the caller identity as `*authored by: <agent> / <model>*`.".to_string(),
            param_type: "string".to_string(),
            required: true,
        });
        parameters.extend(super::identity_params());
        ToolSchema {
            name: "orbit.duel.plan.add".to_string(),
            description: "Persist one planning-duel proposal under `planning-duel/<agent>-<model>.md` for the calling agent/model.".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(
            ctx,
            build_update_input(ctx, &input)?,
            OrbitBuiltinAction::TaskUpdate,
        )
    }
}
