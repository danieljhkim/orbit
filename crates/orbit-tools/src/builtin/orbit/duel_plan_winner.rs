use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitDuelPlanWinnerTool;

fn build_update_input(ctx: &ToolContext, input: &Value) -> Result<Value, OrbitError> {
    let identity = super::resolve_identity(ctx, input)?;
    let arbiter_agent = identity.agent.clone().ok_or_else(|| {
        OrbitError::InvalidInput(
            "orbit.duel.plan.winner requires agent identity to record the arbiter".to_string(),
        )
    })?;
    let arbiter_model = identity.model.clone().ok_or_else(|| {
        OrbitError::InvalidInput(
            "orbit.duel.plan.winner requires model identity to record the arbiter".to_string(),
        )
    })?;
    let id = super::required_string(input, &["id"], "id")?;
    let winner_agent_cli =
        super::required_string(input, &["winner_agent_cli"], "winner_agent_cli")?;
    let winner_model = super::required_string(input, &["winner_model"], "winner_model")?;
    let arbiter_rationale = super::required_string(
        input,
        &["arbiter_rationale", "rationale"],
        "arbiter_rationale",
    )?;
    let winner_payload = json!({
        "winner_agent_cli": winner_agent_cli,
        "winner_model": winner_model,
        "artifact_path": format!("planning-duel/{}-{}.md", winner_agent_cli, winner_model),
        "arbiter_agent_cli": arbiter_agent,
        "arbiter_model": arbiter_model,
        "arbiter_rationale": arbiter_rationale,
    });
    Ok(json!({
        "id": id,
        "artifacts": [{
            "path": "planning-duel/winner.json",
            "content": serde_json::to_string(&winner_payload).expect("winner payload serializes"),
        }],
        "agent": identity.agent,
        "model": identity.model,
    }))
}

impl Tool for OrbitDuelPlanWinnerTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::orbit_id_params("task");
        parameters.extend([
            ToolParam {
                name: "winner_agent_cli".to_string(),
                description: "Agent CLI family parsed from the winning planner artifact signature."
                    .to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "winner_model".to_string(),
                description: "Model parsed from the winning planner artifact signature."
                    .to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "arbiter_rationale".to_string(),
                description: "Short explanation of why the selected plan won.".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
        ]);
        parameters.extend(super::identity_params());
        ToolSchema {
            name: "orbit.duel.plan.winner".to_string(),
            description:
                "Persist the planning-duel winner marker under `planning-duel/winner.json`."
                    .to_string(),
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
