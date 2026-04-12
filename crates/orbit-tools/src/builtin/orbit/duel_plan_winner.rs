use orbit_exec::{NoSandbox, run_process};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::json;

use crate::{Tool, ToolContext};

pub struct OrbitDuelPlanWinnerTool;

fn build_exec_requests(
    ctx: &ToolContext,
    input: &serde_json::Value,
) -> Result<(orbit_exec::ExecRequest, orbit_exec::ExecRequest), OrbitError> {
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
    let update_input = json!({
        "id": id,
        "artifacts": [{
            "path": "planning-duel/winner.json",
            "content": serde_json::to_string(&winner_payload).expect("winner payload serializes"),
        }],
        "agent": identity.agent,
        "model": identity.model,
    });
    super::task_update::build_exec_requests(ctx, &update_input)
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

    fn execute(
        &self,
        ctx: &ToolContext,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, OrbitError> {
        let (update_req, show_req) = build_exec_requests(ctx, &input)?;
        let update_result = run_process(&update_req, &NoSandbox)?;
        if !update_result.success {
            let stderr = update_result.stderr.trim();
            let detail = if stderr.is_empty() {
                "command returned non-zero exit status"
            } else {
                stderr
            };
            return Err(OrbitError::Execution(format!(
                "orbit duel plan winner failed: {detail}"
            )));
        }
        super::run_orbit_json_command(show_req, "orbit task show")
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::Value;
    use serde_json::json;

    use crate::ToolContext;

    use super::build_exec_requests;

    fn test_context() -> ToolContext {
        ToolContext {
            cwd: Some("/tmp/orbit".to_string()),
            orbit_root: Some(PathBuf::from("/tmp/orbit-root")),
            agent_name: Some("gemini".to_string()),
            model_name: Some("gemini-3.1-pro-preview".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn build_exec_requests_serializes_winner_json_payload() {
        let (update, _) = build_exec_requests(
            &test_context(),
            &json!({
                "id": "T20260412-0759",
                "winner_agent_cli": "codex",
                "winner_model": "gpt-5.4",
                "arbiter_rationale": "More concrete writeback and test coverage."
            }),
        )
        .expect("request should build");

        let artifact_value = update
            .args
            .iter()
            .enumerate()
            .find_map(|(index, arg)| {
                (arg == "--artifact")
                    .then(|| update.args.get(index + 1))
                    .flatten()
            })
            .expect("artifact arg");
        let (path, content) = artifact_value
            .split_once('=')
            .expect("artifact path separator");
        assert_eq!(path, "planning-duel/winner.json");
        let payload = serde_json::from_str::<Value>(content).expect("winner json");
        assert_eq!(payload["winner_agent_cli"], json!("codex"));
        assert_eq!(payload["winner_model"], json!("gpt-5.4"));
        assert_eq!(
            payload["artifact_path"],
            json!("planning-duel/codex-gpt-5.4.md")
        );
        assert_eq!(payload["arbiter_agent_cli"], json!("gemini"));
        assert_eq!(payload["arbiter_model"], json!("gemini-3.1-pro-preview"));
    }
}
