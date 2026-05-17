// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use orbit_common::types::{AgentFamily, OrbitError, RoleSlot, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitDuelPlanWinnerTool;

fn build_update_input(ctx: &ToolContext, input: &Value) -> Result<Value, OrbitError> {
    let identity = super::super::resolve_identity(ctx, input)?;
    let arbiter_family = identity.agent.as_deref().ok_or_else(|| {
        OrbitError::InvalidInput(
            "orbit.duel.plan.winner requires agent identity to record the arbiter".to_string(),
        )
    })?;
    let arbiter_family = AgentFamily::parse(arbiter_family)?;
    let id = super::super::required_string(input, &["id"], "id")?;
    let winner_slot = input
        .get("winner_slot")
        .or_else(|| input.get("winner_role_slot"))
        .and_then(Value::as_str)
        .map(str::parse::<RoleSlot>)
        .transpose()?;
    let winner_agent_cli = super::super::optional_string_alias(input, &["winner_agent_cli"])?;
    let winner_model = super::super::optional_string_alias(input, &["winner_model"])?;
    if winner_slot.is_none() && (winner_agent_cli.is_none() || winner_model.is_none()) {
        return Err(OrbitError::InvalidInput(
            "orbit.duel.plan.winner requires winner_slot (or legacy winner_agent_cli and winner_model)"
                .to_string(),
        ));
    }
    let arbiter_rationale = super::super::required_string(
        input,
        &["arbiter_rationale", "rationale"],
        "arbiter_rationale",
    )?;
    let mut winner_payload = serde_json::Map::new();
    if let Some(slot) = winner_slot {
        if slot.planner_slot().is_none() {
            return Err(OrbitError::InvalidInput(
                "winner_slot must be planner_a or planner_b".to_string(),
            ));
        }
        winner_payload.insert(
            "winner_slot".to_string(),
            Value::String(slot.as_str().to_string()),
        );
        winner_payload.insert(
            "artifact_path".to_string(),
            Value::String(format!("planning-duel/{}.md", slot.as_str())),
        );
    }
    if let Some(winner_agent_cli) = winner_agent_cli {
        winner_payload.insert(
            "winner_agent_cli".to_string(),
            Value::String(winner_agent_cli),
        );
    }
    if let Some(winner_model) = winner_model {
        winner_payload.insert("winner_model".to_string(), Value::String(winner_model));
    }
    winner_payload.insert(
        "arbiter_family".to_string(),
        Value::String(arbiter_family.as_str().to_string()),
    );
    winner_payload.insert(
        "arbiter_rationale".to_string(),
        Value::String(arbiter_rationale),
    );
    Ok(json!({
        "id": id,
        "artifacts": [{
            "path": "planning-duel/winner.json",
            "content": serde_json::to_string(&winner_payload).expect("winner payload serializes"),
        }],
        "agent": arbiter_family.as_str(),
        "model": arbiter_family.as_str(),
    }))
}

impl Tool for OrbitDuelPlanWinnerTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::super::orbit_id_params("task");
        parameters.extend([
            ToolParam {
                name: "winner_slot".to_string(),
                description: "Winning planner slot: planner_a or planner_b.".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "winner_agent_cli".to_string(),
                description: "Deprecated legacy winner family field; prefer winner_slot."
                    .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "winner_model".to_string(),
                description: "Deprecated legacy winner model field; prefer winner_slot."
                    .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "arbiter_rationale".to_string(),
                description: "Short explanation of why the selected plan won.".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
        ]);
        parameters.extend(super::super::identity_params());
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
        super::super::execute_host_action(
            ctx,
            build_update_input(ctx, &input)?,
            OrbitBuiltinAction::TaskUpdate,
        )
    }
}
