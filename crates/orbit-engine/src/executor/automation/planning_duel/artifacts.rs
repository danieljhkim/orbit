use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_tools::ToolContext;
use orbit_types::{
    OrbitError, PlannerSlot, PlanningRoleAssignment, PlanningRoles, Role, TaskArtifact, TaskComment,
};
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};
use crate::executor::automation::input::required_input_string;

use super::roles::parse_planning_duel_roles;
use super::types::{PlanningDuelPlanArtifact, PlanningDuelWinnerArtifact};

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

pub(super) fn winner_artifact_from_artifacts(
    artifacts: &[TaskArtifact],
) -> Result<PlanningDuelWinnerArtifact, OrbitError> {
    let winner_artifact = artifacts
        .iter()
        .find(|artifact| artifact.path == WINNER_ARTIFACT_PATH)
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "missing required task artifact `{WINNER_ARTIFACT_PATH}`"
            ))
        })?;
    serde_json::from_str::<PlanningDuelWinnerArtifact>(&winner_artifact.content).map_err(|err| {
        OrbitError::InvalidInput(format!("invalid `{WINNER_ARTIFACT_PATH}` payload: {err}"))
    })
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
        orbit_root: Some(host.data_root().to_path_buf()),
        task_id: Some(task_id.to_string()),
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

pub(super) fn writeback_planning_duel_task<H: TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let artifacts = host.get_task_artifacts(task_id)?;
    let winner = winner_artifact_from_artifacts(&artifacts)?;
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
    let winner_slot = if input.get("planning_duel_roles").is_some() {
        let roles = parse_planning_duel_roles(input)?;
        if winner.arbiter_agent_cli != roles.arbiter.agent
            || winner.arbiter_model != roles.arbiter.model
        {
            return Err(OrbitError::InvalidInput(format!(
                "winner artifact arbiter {}/{} does not match recorded arbiter {}/{}",
                winner.arbiter_agent_cli,
                winner.arbiter_model,
                roles.arbiter.agent,
                roles.arbiter.model
            )));
        }
        Some(winner_slot_for_assignment(&roles, &winner_assignment)?)
    } else {
        None
    };
    let winning_plan = normalize_winning_plan_for_task(&winning_artifact.content);
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
        "Planning duel resolved.\n\nWinner: {winner_label} ({}/{})\n\nRationale: {}\n\nWinning plan persisted to task.plan. Task status was left unchanged.",
        winner_assignment.agent, winner_assignment.model, winner.arbiter_rationale
    );

    host.apply_task_automation_update(
        task_id,
        TaskAutomationUpdate {
            plan: Some(winning_plan),
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
        "status_unchanged": true,
        "winner_agent_cli": winner_assignment.agent,
        "winner_model": winner_assignment.model,
    }))
}
