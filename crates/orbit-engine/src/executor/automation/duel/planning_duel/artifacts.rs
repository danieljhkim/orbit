use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_common::types::{
    OrbitError, OrbitEvent, PlannerSlot, PlanningRoleAssignment, PlanningRoles, Role, TaskArtifact,
    TaskComment,
};
use orbit_tools::ToolContext;
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};
use crate::executor::automation::input::required_input_string;

use super::context_files::extract_context_files_from_plan;
use super::roles::parse_planning_duel_roles;
use super::types::{
    PlanningDuelPlanArtifact, PlanningDuelWinnerArtifact, PlanningDuelWinnerMarker,
};

const PLANNING_DUEL_ARTIFACT_PREFIX: &str = "planning-duel/";
const PLANNING_DUEL_PLAN_EXTENSION: &str = ".md";
const WINNER_ARTIFACT_PATH: &str = "planning-duel/winner.json";
const TASKS_DIR_NAME: &str = "tasks";
const TASK_ARTIFACTS_DIR_NAME: &str = "artifacts";
const AUTHOR_SIGNATURE_PREFIX: &str = "*authored by: ";
const AUTHOR_SIGNATURE_SEPARATOR: &str = " / ";

pub(super) fn parse_planning_duel_signature(
    content: &str,
) -> Result<PlanningRoleAssignment, OrbitError> {
    let first_line = content
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .ok_or_else(|| {
            OrbitError::InvalidInput(
                "planning duel artifact must start with an authored-by signature line".to_string(),
            )
        })?;
    let signature = first_line
        .strip_prefix(AUTHOR_SIGNATURE_PREFIX)
        .and_then(|value| value.strip_suffix('*'))
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "planning duel artifact signature must match `{AUTHOR_SIGNATURE_PREFIX}<agent> / <model>*`"
            ))
        })?;
    let (agent, model) = signature
        .split_once(AUTHOR_SIGNATURE_SEPARATOR)
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "planning duel artifact signature must contain `{AUTHOR_SIGNATURE_SEPARATOR}`"
            ))
        })?;
    if agent.trim().is_empty() || model.trim().is_empty() {
        return Err(OrbitError::InvalidInput(
            "planning duel artifact signature must include both agent and model".to_string(),
        ));
    }
    Ok(PlanningRoleAssignment {
        agent: agent.trim().to_string(),
        model: model.trim().to_string(),
    })
}

pub(super) fn planning_duel_plan_artifacts(
    artifacts: &[TaskArtifact],
) -> Result<Vec<PlanningDuelPlanArtifact>, OrbitError> {
    let mut plan_artifacts = artifacts
        .iter()
        .filter(|artifact| {
            artifact.path.starts_with(PLANNING_DUEL_ARTIFACT_PREFIX)
                && artifact.path.ends_with(PLANNING_DUEL_PLAN_EXTENSION)
        })
        .map(|artifact| {
            Ok(PlanningDuelPlanArtifact {
                path: artifact.path.clone(),
                content: artifact.content.clone(),
                author: parse_planning_duel_signature(&artifact.content)?,
            })
        })
        .collect::<Result<Vec<_>, OrbitError>>()?;
    plan_artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    if plan_artifacts.is_empty() {
        return Err(OrbitError::InvalidInput(
            "missing planning duel markdown artifacts under planning-duel/".to_string(),
        ));
    }
    Ok(plan_artifacts)
}

pub(super) fn plan_artifact_for_assignment<'a>(
    plan_artifacts: &'a [PlanningDuelPlanArtifact],
    assignment: &PlanningRoleAssignment,
) -> Result<&'a PlanningDuelPlanArtifact, OrbitError> {
    let matches = plan_artifacts
        .iter()
        .filter(|artifact| artifact.author == *assignment)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [artifact] => Ok(*artifact),
        [] => Err(OrbitError::InvalidInput(format!(
            "missing planning duel artifact for {}/{}",
            assignment.agent, assignment.model
        ))),
        _ => Err(OrbitError::InvalidInput(format!(
            "found multiple planning duel artifacts for {}/{}",
            assignment.agent, assignment.model
        ))),
    }
}

pub(super) fn plan_artifact_by_path<'a>(
    plan_artifacts: &'a [PlanningDuelPlanArtifact],
    artifact_path: &str,
) -> Result<&'a PlanningDuelPlanArtifact, OrbitError> {
    let matches = plan_artifacts
        .iter()
        .filter(|artifact| artifact.path == artifact_path)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [artifact] => Ok(*artifact),
        [] => Err(OrbitError::InvalidInput(format!(
            "missing planning duel artifact `{artifact_path}`"
        ))),
        _ => Err(OrbitError::InvalidInput(format!(
            "found multiple planning duel artifacts at `{artifact_path}`"
        ))),
    }
}

fn required_winner_marker_field(value: &str, field: &str) -> Result<String, OrbitError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "planning duel winner marker field `{field}` must not be empty"
        )));
    }
    Ok(value.to_string())
}

fn optional_winner_marker_field(
    value: Option<String>,
    field: &str,
) -> Result<Option<String>, OrbitError> {
    value
        .map(|value| required_winner_marker_field(&value, field))
        .transpose()
}

fn arbiter_identity_from_marker(
    marker_agent: Option<String>,
    marker_model: Option<String>,
    roles: Option<&PlanningRoles>,
) -> Result<PlanningRoleAssignment, OrbitError> {
    match roles {
        Some(roles) => {
            if let Some(agent) = marker_agent.as_deref()
                && agent != roles.arbiter.agent.as_str()
            {
                return Err(OrbitError::InvalidInput(format!(
                    "winner artifact arbiter {}/{} does not match recorded arbiter {}/{}",
                    agent,
                    marker_model.as_deref().unwrap_or("<unspecified>"),
                    roles.arbiter.agent,
                    roles.arbiter.model
                )));
            }
            if let Some(model) = marker_model.as_deref()
                && model != roles.arbiter.model.as_str()
            {
                return Err(OrbitError::InvalidInput(format!(
                    "winner artifact arbiter {}/{} does not match recorded arbiter {}/{}",
                    marker_agent.as_deref().unwrap_or("<unspecified>"),
                    model,
                    roles.arbiter.agent,
                    roles.arbiter.model
                )));
            }
            Ok(roles.arbiter.clone())
        }
        None => Ok(PlanningRoleAssignment {
            agent: marker_agent.ok_or_else(|| {
                OrbitError::InvalidInput(
                    "planning duel winner marker requires `arbiter_agent_cli` when `planning_duel_roles` are unavailable".to_string(),
                )
            })?,
            model: marker_model.ok_or_else(|| {
                OrbitError::InvalidInput(
                    "planning duel winner marker requires `arbiter_model` when `planning_duel_roles` are unavailable".to_string(),
                )
            })?,
        }),
    }
}

fn normalize_winner_marker(
    marker: PlanningDuelWinnerMarker,
    plan_artifacts: &[PlanningDuelPlanArtifact],
    roles: Option<&PlanningRoles>,
) -> Result<PlanningDuelWinnerArtifact, OrbitError> {
    let PlanningDuelWinnerMarker {
        winner_agent_cli,
        winner_model,
        artifact_path,
        arbiter_agent_cli,
        arbiter_model,
        arbiter_rationale,
    } = marker;

    let winner_agent_cli = required_winner_marker_field(&winner_agent_cli, "winner_agent_cli")?;
    let winner_model = required_winner_marker_field(&winner_model, "winner_model")?;
    let arbiter_rationale = required_winner_marker_field(&arbiter_rationale, "arbiter_rationale")?;
    let winner_assignment = PlanningRoleAssignment {
        agent: winner_agent_cli.clone(),
        model: winner_model.clone(),
    };

    let artifact_path = match optional_winner_marker_field(artifact_path, "artifact_path")? {
        Some(artifact_path) => {
            let winning_artifact = plan_artifact_by_path(plan_artifacts, &artifact_path)?;
            if winning_artifact.author != winner_assignment {
                return Err(OrbitError::InvalidInput(format!(
                    "winner artifact `{}` is authored by {}/{} instead of declared winner {}/{}",
                    artifact_path,
                    winning_artifact.author.agent,
                    winning_artifact.author.model,
                    winner_assignment.agent,
                    winner_assignment.model
                )));
            }
            artifact_path
        }
        None => plan_artifact_for_assignment(plan_artifacts, &winner_assignment)?
            .path
            .clone(),
    };

    let arbiter_agent_cli = optional_winner_marker_field(arbiter_agent_cli, "arbiter_agent_cli")?;
    let arbiter_model = optional_winner_marker_field(arbiter_model, "arbiter_model")?;
    let arbiter = arbiter_identity_from_marker(arbiter_agent_cli, arbiter_model, roles)?;

    Ok(PlanningDuelWinnerArtifact {
        winner_agent_cli,
        winner_model,
        artifact_path,
        arbiter_agent_cli: arbiter.agent,
        arbiter_model: arbiter.model,
        arbiter_rationale,
    })
}

pub(super) fn winner_artifact_from_artifacts(
    artifacts: &[TaskArtifact],
    roles: Option<&PlanningRoles>,
) -> Result<PlanningDuelWinnerArtifact, OrbitError> {
    let winner_artifact = artifacts
        .iter()
        .find(|artifact| artifact.path == WINNER_ARTIFACT_PATH)
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "missing required task artifact `{WINNER_ARTIFACT_PATH}`"
            ))
        })?;
    let marker = serde_json::from_str::<PlanningDuelWinnerMarker>(&winner_artifact.content)
        .map_err(|err| {
            OrbitError::InvalidInput(format!(
                "invalid `{WINNER_ARTIFACT_PATH}` marker payload: {err}"
            ))
        })?;
    let plan_artifacts = planning_duel_plan_artifacts(artifacts)?;
    normalize_winner_marker(marker, &plan_artifacts, roles)
}

pub(super) fn winner_assignment(winner: &PlanningDuelWinnerArtifact) -> PlanningRoleAssignment {
    PlanningRoleAssignment {
        agent: winner.winner_agent_cli.clone(),
        model: winner.winner_model.clone(),
    }
}

pub(super) fn winner_slot_for_assignment(
    roles: &PlanningRoles,
    winner: &PlanningRoleAssignment,
) -> Result<PlannerSlot, OrbitError> {
    if roles.planner_a == *winner {
        return Ok(PlannerSlot::PlannerA);
    }
    if roles.planner_b == *winner {
        return Ok(PlannerSlot::PlannerB);
    }
    Err(OrbitError::InvalidInput(format!(
        "winner {}/{} does not match the current planner assignments",
        winner.agent, winner.model
    )))
}

pub(super) fn normalize_winning_plan_for_task(content: &str) -> String {
    let lines = content.lines().collect::<Vec<_>>();
    if lines.len() <= 1 {
        return content.trim().to_string();
    }
    if parse_planning_duel_signature(content).is_ok() {
        return lines[1..].join("\n").trim().to_string();
    }
    content.trim().to_string()
}

fn find_task_dir(tasks_root: &Path, task_id: &str) -> Result<Option<PathBuf>, OrbitError> {
    if !tasks_root.exists() {
        return Ok(None);
    }

    let mut pending = vec![tasks_root.to_path_buf()];
    let mut matches = Vec::new();

    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).map_err(|err| OrbitError::Io(err.to_string()))? {
            let entry = entry.map_err(|err| OrbitError::Io(err.to_string()))?;
            let path = entry.path();
            if !entry
                .file_type()
                .map_err(|err| OrbitError::Io(err.to_string()))?
                .is_dir()
            {
                continue;
            }

            if path.file_name().and_then(|name| name.to_str()) == Some(task_id) {
                matches.push(path);
                continue;
            }

            pending.push(path);
        }
    }

    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.pop()),
        _ => Err(OrbitError::Execution(format!(
            "found multiple task directories for '{task_id}' while cleaning planning duel artifacts"
        ))),
    }
}

pub(super) fn cleanup_stale_planning_duel_artifacts<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    task_id: &str,
) -> Result<(), OrbitError> {
    let stale_artifacts = host
        .get_task_artifacts(task_id)?
        .into_iter()
        .filter(|artifact| artifact.path.starts_with(PLANNING_DUEL_ARTIFACT_PREFIX))
        .collect::<Vec<_>>();
    if stale_artifacts.is_empty() {
        return Ok(());
    }

    let tasks_root = host.data_root().join(TASKS_DIR_NAME);
    let task_dir = find_task_dir(&tasks_root, task_id)?.ok_or_else(|| {
        OrbitError::Execution(format!(
            "could not locate task directory for '{task_id}' while cleaning stale planning duel artifacts"
        ))
    })?;
    let artifacts_root = task_dir.join(TASK_ARTIFACTS_DIR_NAME);
    let tool_context = ToolContext {
        cwd: Some(host.data_root().display().to_string()),
        allowed_tools: vec!["fs.delete".to_string()],
        workspace_root: Some(host.data_root().to_path_buf()),
        ..ToolContext::default()
    };

    for artifact in stale_artifacts {
        let artifact_path = artifacts_root.join(&artifact.path);
        if !artifact_path.exists() {
            return Err(OrbitError::Execution(format!(
                "stale planning duel artifact '{}' is missing on disk for task '{}'",
                artifact.path, task_id
            )));
        }
        let _ = host.run_tool_with_context_and_role(
            "fs.delete",
            json!({ "path": artifact_path.display().to_string() }),
            Role::Admin,
            tool_context.clone(),
        )?;
    }

    Ok(())
}

pub(super) fn writeback_planning_duel_task<H: TaskHost + RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let current_status = host.get_task(task_id)?.status.to_string();
    let artifacts = host.get_task_artifacts(task_id)?;
    let roles = input
        .get("planning_duel_roles")
        .map(|_| parse_planning_duel_roles(input))
        .transpose()?;
    let winner = winner_artifact_from_artifacts(&artifacts, roles.as_ref())?;
    let winner_assignment = winner_assignment(&winner);
    let plan_artifacts = planning_duel_plan_artifacts(&artifacts)?;
    let winning_artifact = plan_artifact_by_path(&plan_artifacts, &winner.artifact_path)?;
    if winning_artifact.author != winner_assignment {
        return Err(OrbitError::InvalidInput(format!(
            "winner artifact `{}` is authored by {}/{} instead of declared winner {}/{}",
            winner.artifact_path,
            winning_artifact.author.agent,
            winning_artifact.author.model,
            winner_assignment.agent,
            winner_assignment.model
        )));
    }
    let winner_slot = if let Some(roles) = roles.as_ref() {
        Some(winner_slot_for_assignment(roles, &winner_assignment)?)
    } else {
        None
    };
    let winning_plan = normalize_winning_plan_for_task(&winning_artifact.content);
    let extraction = extract_context_files_from_plan(&winning_plan);
    let context_files = extraction.as_ref().map(|e| e.canonical_entries.clone());
    if let Some(extraction) = extraction.as_ref() {
        for skipped in &extraction.skipped {
            // Best-effort observability — never fail writeback on event-record error.
            let _ = host.record_event(OrbitEvent::PlanningDuelContextFileSkipped {
                task_id: task_id.to_string(),
                raw_entry: skipped.raw_entry.clone(),
                reason: skipped.reason.clone(),
            });
        }
    }

    let winner_label = winner_slot
        .map(|slot| match slot {
            PlannerSlot::PlannerA => "planner_a",
            PlannerSlot::PlannerB => "planner_b",
        })
        .unwrap_or("planner");

    let status_note = format!(
        "planning duel winner={winner_label} ({}/{})",
        winner_assignment.agent, winner_assignment.model
    );
    let comment_message = format!(
        "Planning duel resolved.\n\nWinner: {winner_label} ({}/{})\n\nRationale: {}\n\nWinning plan persisted to task.plan. Task status remains {current_status}.",
        winner_assignment.agent, winner_assignment.model, winner.arbiter_rationale
    );

    host.apply_task_automation_update(
        task_id,
        TaskAutomationUpdate {
            plan: Some(winning_plan),
            context_files,
            status_event: Some("planning_duel_resolved".to_string()),
            status_note: Some(format!(
                "{status_note}; rationale={}",
                winner.arbiter_rationale
            )),
            append_comments: vec![TaskComment {
                at: Utc::now(),
                by: winner.arbiter_agent_cli.clone(),
                message: comment_message,
            }],
            agent: Some(winner_assignment.agent.clone()),
            model: Some(winner_assignment.model.clone()),
            ..TaskAutomationUpdate::default()
        },
    )?;

    Ok(json!({
        "task_id": task_id,
        "task_status": host.get_task(task_id)?.status.to_string(),
        "winner_agent_cli": winner_assignment.agent,
        "winner_model": winner_assignment.model,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_common::types::{PlanningRoleAssignment, PlanningRoles, TaskArtifact};
    use serde_json::{Value, json};

    fn task_artifact(path: &str, content: String) -> TaskArtifact {
        TaskArtifact {
            path: path.to_string(),
            content,
        }
    }

    fn plan_artifact(path: &str, agent: &str, model: &str) -> TaskArtifact {
        task_artifact(
            path,
            format!("*authored by: {agent} / {model}*\n## Plan\nDo the thing.\n"),
        )
    }

    fn winner_marker(payload: Value) -> TaskArtifact {
        task_artifact(WINNER_ARTIFACT_PATH, payload.to_string())
    }

    fn planning_roles() -> PlanningRoles {
        PlanningRoles {
            planner_a: PlanningRoleAssignment {
                agent: "codex".to_string(),
                model: "gpt-5.5".to_string(),
            },
            planner_b: PlanningRoleAssignment {
                agent: "claude".to_string(),
                model: "claude-opus-4-7".to_string(),
            },
            arbiter: PlanningRoleAssignment {
                agent: "gemini".to_string(),
                model: "gemini-3.1-pro".to_string(),
            },
        }
    }

    fn planning_duel_artifacts(winner_payload: Value) -> Vec<TaskArtifact> {
        vec![
            plan_artifact("planning-duel/codex-gpt-5.5.md", "codex", "gpt-5.5"),
            plan_artifact(
                "planning-duel/claude-claude-opus-4-7.md",
                "claude",
                "claude-opus-4-7",
            ),
            winner_marker(winner_payload),
        ]
    }

    fn invalid_input_message(error: OrbitError) -> String {
        match error {
            OrbitError::InvalidInput(message) => message,
            other => panic!("expected invalid input, got {other:?}"),
        }
    }

    #[test]
    fn planning_duel_winner_marker_omits_derived_fields_when_roles_available() {
        let roles = planning_roles();
        let artifacts = planning_duel_artifacts(json!({
            "id": "T20260427-47",
            "winner_agent_cli": "claude",
            "winner_model": "claude-opus-4-7",
            "arbiter_rationale": "Claude provided a more comprehensive diagnosis."
        }));

        let winner = winner_artifact_from_artifacts(&artifacts, Some(&roles))
            .expect("minimal winner marker should normalize");

        assert_eq!(winner.winner_agent_cli, "claude");
        assert_eq!(winner.winner_model, "claude-opus-4-7");
        assert_eq!(
            winner.artifact_path,
            "planning-duel/claude-claude-opus-4-7.md"
        );
        assert_eq!(winner.arbiter_agent_cli, "gemini");
        assert_eq!(winner.arbiter_model, "gemini-3.1-pro");
        assert_eq!(
            winner.arbiter_rationale,
            "Claude provided a more comprehensive diagnosis."
        );
    }

    #[test]
    fn planning_duel_winner_marker_rejects_explicit_arbiter_mismatch() {
        let roles = planning_roles();
        let artifacts = planning_duel_artifacts(json!({
            "winner_agent_cli": "claude",
            "winner_model": "claude-opus-4-7",
            "arbiter_agent_cli": "codex",
            "arbiter_model": "gpt-5.5",
            "arbiter_rationale": "Claude provided a more comprehensive diagnosis."
        }));

        let message = invalid_input_message(
            winner_artifact_from_artifacts(&artifacts, Some(&roles))
                .expect_err("arbiter mismatch should be rejected"),
        );

        assert!(
            message.contains(
                "winner artifact arbiter codex/gpt-5.5 does not match recorded arbiter gemini/gemini-3.1-pro"
            ),
            "{message}"
        );
    }

    #[test]
    fn planning_duel_winner_marker_requires_arbiter_identity_without_roles() {
        let artifacts = planning_duel_artifacts(json!({
            "winner_agent_cli": "claude",
            "winner_model": "claude-opus-4-7",
            "arbiter_rationale": "Claude provided a more comprehensive diagnosis."
        }));

        let message = invalid_input_message(
            winner_artifact_from_artifacts(&artifacts, None)
                .expect_err("arbiter identity cannot be inferred without roles"),
        );

        assert!(
            message.contains(
                "planning duel winner marker requires `arbiter_agent_cli` when `planning_duel_roles` are unavailable"
            ),
            "{message}"
        );
    }

    #[test]
    fn planning_duel_winner_marker_accepts_legacy_full_payload_without_roles() {
        let artifacts = planning_duel_artifacts(json!({
            "winner_agent_cli": "claude",
            "winner_model": "claude-opus-4-7",
            "artifact_path": "planning-duel/claude-claude-opus-4-7.md",
            "arbiter_agent_cli": "gemini",
            "arbiter_model": "gemini-3.1-pro",
            "arbiter_rationale": "Claude provided a more comprehensive diagnosis."
        }));

        let winner = winner_artifact_from_artifacts(&artifacts, None)
            .expect("legacy full winner payload should still normalize");

        assert_eq!(winner.winner_agent_cli, "claude");
        assert_eq!(winner.winner_model, "claude-opus-4-7");
        assert_eq!(
            winner.artifact_path,
            "planning-duel/claude-claude-opus-4-7.md"
        );
        assert_eq!(winner.arbiter_agent_cli, "gemini");
        assert_eq!(winner.arbiter_model, "gemini-3.1-pro");
    }
}
