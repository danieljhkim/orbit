use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_common::types::{
    AgentFamily, OrbitError, OrbitEvent, PlanningRoleAssignment, PlanningRoles, Role, RoleSlot,
    TaskArtifact, TaskComment,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PlanningDuelSignature {
    pub family: AgentFamily,
    pub slot: RoleSlot,
}

pub(super) fn parse_planning_duel_signature(
    content: &str,
) -> Result<PlanningDuelSignature, OrbitError> {
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
                "planning duel artifact signature must match `{AUTHOR_SIGNATURE_PREFIX}<family> / <slot>*`"
            ))
        })?;
    let (family, slot) = signature
        .split_once(AUTHOR_SIGNATURE_SEPARATOR)
        .ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "planning duel artifact signature must contain `{AUTHOR_SIGNATURE_SEPARATOR}`"
            ))
        })?;
    if family.trim().is_empty() || slot.trim().is_empty() {
        return Err(OrbitError::InvalidInput(
            "planning duel artifact signature must include both family and slot".to_string(),
        ));
    }
    Ok(PlanningDuelSignature {
        family: family.trim().parse()?,
        slot: slot.trim().parse()?,
    })
}

fn parse_legacy_planning_duel_signature(
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
            OrbitError::InvalidInput(
                "legacy planning duel artifact signature is malformed".to_string(),
            )
        })?;
    let (agent, _) = signature
        .split_once(AUTHOR_SIGNATURE_SEPARATOR)
        .ok_or_else(|| {
            OrbitError::InvalidInput(
                "legacy planning duel artifact signature must contain agent and model".to_string(),
            )
        })?;
    Ok(PlanningRoleAssignment {
        family: agent.trim().parse()?,
    })
}

fn role_slot_from_artifact_path(path: &str) -> Option<RoleSlot> {
    let name = path
        .strip_prefix(PLANNING_DUEL_ARTIFACT_PREFIX)?
        .strip_suffix(PLANNING_DUEL_PLAN_EXTENSION)?;
    name.parse().ok()
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
            let content = artifact.text_content().ok_or_else(|| {
                OrbitError::InvalidInput(format!(
                    "planning duel artifact '{}' is not valid UTF-8",
                    artifact.path
                ))
            })?;
            let parsed = parse_planning_duel_signature(content);
            let (author, slot) = match parsed {
                Ok(signature) => (
                    PlanningRoleAssignment {
                        family: signature.family,
                    },
                    Some(signature.slot),
                ),
                Err(_) => (
                    parse_legacy_planning_duel_signature(content)?,
                    role_slot_from_artifact_path(&artifact.path),
                ),
            };
            Ok(PlanningDuelPlanArtifact {
                path: artifact.path.clone(),
                content: content.to_string(),
                author,
                slot,
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
    expected_slot: RoleSlot,
) -> Result<&'a PlanningDuelPlanArtifact, OrbitError> {
    let matches = plan_artifacts
        .iter()
        .filter(|artifact| artifact.slot == Some(expected_slot))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [artifact] if artifact.author.family == assignment.family => Ok(*artifact),
        [artifact] => Err(OrbitError::InvalidInput(format!(
            "planning duel artifact for slot {expected_slot} has family {} but expected {}",
            artifact.author.family, assignment.family
        ))),
        [] => Err(OrbitError::InvalidInput(format!(
            "missing planning duel artifact for {} / {}",
            assignment.family, expected_slot
        ))),
        _ => Err(OrbitError::InvalidInput(format!(
            "found multiple planning duel artifacts for slot {expected_slot}"
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

fn assignment_for_slot(roles: &PlanningRoles, slot: RoleSlot) -> &PlanningRoleAssignment {
    match slot {
        RoleSlot::PlannerA => &roles.planner_a,
        RoleSlot::PlannerB => &roles.planner_b,
        RoleSlot::Arbiter => &roles.arbiter,
    }
}

fn family_from_legacy_identity(
    value: Option<String>,
    field: &str,
) -> Result<Option<AgentFamily>, OrbitError> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| required_winner_marker_field(&value, field))
        .transpose()?
        .map(|value| value.parse())
        .transpose()
}

fn normalize_winner_marker(
    marker: PlanningDuelWinnerMarker,
    plan_artifacts: &[PlanningDuelPlanArtifact],
    roles: Option<&PlanningRoles>,
) -> Result<PlanningDuelWinnerArtifact, OrbitError> {
    let PlanningDuelWinnerMarker {
        winner_slot,
        winner_agent_cli,
        winner_model: _,
        artifact_path,
        arbiter_agent_cli,
        arbiter_model: _,
        arbiter_family,
        arbiter_rationale,
    } = marker;

    let arbiter_rationale = required_winner_marker_field(&arbiter_rationale, "arbiter_rationale")?;
    let legacy_winner_family =
        family_from_legacy_identity(Some(winner_agent_cli), "winner_agent_cli")?;
    let winner_slot = winner_slot.or_else(|| {
        artifact_path
            .as_deref()
            .and_then(role_slot_from_artifact_path)
    });
    let (winner_family, winner_slot) = match (roles, winner_slot, legacy_winner_family) {
        (Some(roles), Some(slot), legacy_family) => {
            let family = assignment_for_slot(roles, slot).family;
            if let Some(legacy_family) = legacy_family
                && legacy_family != family
            {
                return Err(OrbitError::InvalidInput(format!(
                    "winner artifact family {legacy_family} does not match recorded {slot} family {family}"
                )));
            }
            (family, Some(slot))
        }
        (Some(roles), None, Some(legacy_family)) => {
            let assignment = PlanningRoleAssignment {
                family: legacy_family,
            };
            let slot = winner_slot_for_assignment(roles, &assignment)?;
            (legacy_family, Some(slot))
        }
        (Some(_), None, None) => {
            return Err(OrbitError::InvalidInput(
                "planning duel winner marker requires `winner_slot`".to_string(),
            ));
        }
        (None, Some(slot), Some(legacy_family)) => (legacy_family, Some(slot)),
        (None, Some(slot), None) => {
            let artifact = plan_artifacts
                .iter()
                .find(|artifact| artifact.slot == Some(slot))
                .ok_or_else(|| {
                    OrbitError::InvalidInput(format!(
                        "missing planning duel artifact for winner slot {slot}"
                    ))
                })?;
            (artifact.author.family, Some(slot))
        }
        (None, None, Some(legacy_family)) => (legacy_family, None),
        (None, None, None) => {
            return Err(OrbitError::InvalidInput(
                "planning duel winner marker requires `winner_slot` or legacy `winner_agent_cli`"
                    .to_string(),
            ));
        }
    };

    let artifact_path = match optional_winner_marker_field(artifact_path, "artifact_path")? {
        Some(artifact_path) => {
            let winning_artifact = plan_artifact_by_path(plan_artifacts, &artifact_path)?;
            if winning_artifact.author.family != winner_family {
                return Err(OrbitError::InvalidInput(format!(
                    "winner artifact `{}` is authored by {} instead of declared winner {}",
                    artifact_path, winning_artifact.author.family, winner_family
                )));
            }
            artifact_path
        }
        None => {
            let slot = winner_slot.ok_or_else(|| {
                OrbitError::InvalidInput(
                    "planning duel winner marker requires artifact_path when winner_slot is unavailable"
                        .to_string(),
                )
            })?;
            plan_artifact_for_assignment(
                plan_artifacts,
                &PlanningRoleAssignment {
                    family: winner_family,
                },
                slot,
            )?
            .path
            .clone()
        }
    };

    let legacy_arbiter_family =
        family_from_legacy_identity(arbiter_agent_cli, "arbiter_agent_cli")?;
    let arbiter_family = match (roles, arbiter_family.or(legacy_arbiter_family)) {
        (Some(roles), Some(marker_family)) if marker_family != roles.arbiter.family => {
            return Err(OrbitError::InvalidInput(format!(
                "winner artifact arbiter {marker_family} does not match recorded arbiter {}",
                roles.arbiter.family
            )));
        }
        (Some(roles), _) => roles.arbiter.family,
        (None, Some(marker_family)) => marker_family,
        (None, None) => {
            return Err(OrbitError::InvalidInput(
                "planning duel winner marker requires `arbiter_family` when `planning_duel_roles` are unavailable".to_string(),
            ));
        }
    };

    Ok(PlanningDuelWinnerArtifact {
        winner_family,
        winner_slot,
        artifact_path,
        arbiter_family,
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
    let marker_content = winner_artifact.text_content().ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "`{WINNER_ARTIFACT_PATH}` marker payload is not valid UTF-8"
        ))
    })?;
    let marker =
        serde_json::from_str::<PlanningDuelWinnerMarker>(marker_content).map_err(|err| {
            OrbitError::InvalidInput(format!(
                "invalid `{WINNER_ARTIFACT_PATH}` marker payload: {err}"
            ))
        })?;
    let plan_artifacts = planning_duel_plan_artifacts(artifacts)?;
    normalize_winner_marker(marker, &plan_artifacts, roles)
}

pub(super) fn winner_assignment(winner: &PlanningDuelWinnerArtifact) -> PlanningRoleAssignment {
    PlanningRoleAssignment {
        family: winner.winner_family,
    }
}

pub(super) fn winner_slot_for_assignment(
    roles: &PlanningRoles,
    winner: &PlanningRoleAssignment,
) -> Result<RoleSlot, OrbitError> {
    if roles.planner_a == *winner {
        return Ok(RoleSlot::PlannerA);
    }
    if roles.planner_b == *winner {
        return Ok(RoleSlot::PlannerB);
    }
    Err(OrbitError::InvalidInput(format!(
        "winner {} does not match the current planner assignments",
        winner.family
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
    if winning_artifact.author.family != winner_assignment.family {
        return Err(OrbitError::InvalidInput(format!(
            "winner artifact `{}` is authored by {} instead of declared winner {}",
            winner.artifact_path, winning_artifact.author.family, winner_assignment.family
        )));
    }
    let winner_slot = roles
        .as_ref()
        .map(|roles| winner_slot_for_assignment(roles, &winner_assignment))
        .transpose()?
        .or(winner.winner_slot);
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

    let winner_label = winner_slot.map(|slot| slot.as_str()).unwrap_or("planner");

    let status_note = format!(
        "planning duel winner={winner_label} ({})",
        winner_assignment.family
    );
    let comment_message = format!(
        "Planning duel resolved.\n\nWinner: {winner_label} ({})\n\nRationale: {}\n\nWinning plan persisted to task.plan. Task status remains {current_status}.",
        winner_assignment.family, winner.arbiter_rationale
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
                by: winner.arbiter_family.to_string(),
                message: comment_message,
            }],
            agent: Some(winner_assignment.family.to_string()),
            model: Some(winner_assignment.family.to_string()),
            ..TaskAutomationUpdate::default()
        },
    )?;

    Ok(json!({
        "task_id": task_id,
        "task_status": host.get_task(task_id)?.status.to_string(),
        "winner_family": winner_assignment.family,
        "winner_slot": winner_slot.map(|slot| slot.as_str().to_string()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_common::types::{
        AgentFamily, PlanningRoleAssignment, PlanningRoles, RoleSlot, TaskArtifact,
    };
    use serde_json::{Value, json};

    fn task_artifact(path: &str, content: String) -> TaskArtifact {
        TaskArtifact::from_text(path, content)
    }

    fn plan_artifact(path: &str, family: &str, slot: &str) -> TaskArtifact {
        task_artifact(
            path,
            format!("*authored by: {family} / {slot}*\n## Plan\nDo the thing.\n"),
        )
    }

    fn winner_marker(payload: Value) -> TaskArtifact {
        task_artifact(WINNER_ARTIFACT_PATH, payload.to_string())
    }

    fn planning_roles() -> PlanningRoles {
        PlanningRoles {
            planner_a: PlanningRoleAssignment {
                family: AgentFamily::Codex,
            },
            planner_b: PlanningRoleAssignment {
                family: AgentFamily::Claude,
            },
            arbiter: PlanningRoleAssignment {
                family: AgentFamily::Gemini,
            },
        }
    }

    fn planning_duel_artifacts(winner_payload: Value) -> Vec<TaskArtifact> {
        vec![
            plan_artifact("planning-duel/planner_a.md", "codex", "planner_a"),
            plan_artifact("planning-duel/planner_b.md", "claude", "planner_b"),
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
    fn planning_duel_signature_extracts_family_and_slot() {
        let signature = parse_planning_duel_signature("*authored by: gemini / planner_a*\n## Plan")
            .expect("signature parses");
        assert_eq!(signature.family, AgentFamily::Gemini);
        assert_eq!(signature.slot, RoleSlot::PlannerA);

        assert!(parse_planning_duel_signature("*authored by: gemini*\n").is_err());
        assert!(parse_planning_duel_signature("*authored by: / planner_a*\n").is_err());
        assert!(parse_planning_duel_signature("*authored by: pro / planner_a*\n").is_err());
    }

    #[test]
    fn plan_artifact_validation_uses_family_and_slot_not_configured_model() {
        let artifacts = planning_duel_plan_artifacts(&[
            plan_artifact("planning-duel/planner_a.md", "gemini", "planner_a"),
            plan_artifact("planning-duel/planner_b.md", "codex", "planner_b"),
        ])
        .expect("plan artifacts parse");
        let assignment = PlanningRoleAssignment {
            family: AgentFamily::Gemini,
        };

        let artifact = plan_artifact_for_assignment(&artifacts, &assignment, RoleSlot::PlannerA)
            .expect("matching family and slot validate");

        assert_eq!(artifact.path, "planning-duel/planner_a.md");
    }

    #[test]
    fn plan_artifact_validation_reports_family_mismatch() {
        let artifacts = planning_duel_plan_artifacts(&[plan_artifact(
            "planning-duel/planner_a.md",
            "claude",
            "planner_a",
        )])
        .expect("plan artifacts parse");
        let assignment = PlanningRoleAssignment {
            family: AgentFamily::Gemini,
        };

        let message = invalid_input_message(
            plan_artifact_for_assignment(&artifacts, &assignment, RoleSlot::PlannerA)
                .expect_err("mismatched family fails"),
        );

        assert!(message.contains("expected gemini"), "{message}");
        assert!(message.contains("has family claude"), "{message}");
    }

    #[test]
    fn planning_duel_winner_marker_omits_derived_fields_when_roles_available() {
        let roles = planning_roles();
        let artifacts = planning_duel_artifacts(json!({
            "id": "T20260427-47",
            "winner_slot": "planner_b",
            "arbiter_rationale": "Claude provided a more comprehensive diagnosis."
        }));

        let winner = winner_artifact_from_artifacts(&artifacts, Some(&roles))
            .expect("minimal winner marker should normalize");

        assert_eq!(winner.winner_family, AgentFamily::Claude);
        assert_eq!(winner.winner_slot, Some(RoleSlot::PlannerB));
        assert_eq!(winner.artifact_path, "planning-duel/planner_b.md");
        assert_eq!(winner.arbiter_family, AgentFamily::Gemini);
        assert_eq!(
            winner.arbiter_rationale,
            "Claude provided a more comprehensive diagnosis."
        );
    }

    #[test]
    fn planning_duel_winner_marker_rejects_explicit_arbiter_mismatch() {
        let roles = planning_roles();
        let artifacts = planning_duel_artifacts(json!({
            "winner_slot": "planner_b",
            "arbiter_agent_cli": "codex",
            "arbiter_rationale": "Claude provided a more comprehensive diagnosis."
        }));

        let message = invalid_input_message(
            winner_artifact_from_artifacts(&artifacts, Some(&roles))
                .expect_err("arbiter mismatch should be rejected"),
        );

        assert!(
            message
                .contains("winner artifact arbiter codex does not match recorded arbiter gemini"),
            "{message}"
        );
    }

    #[test]
    fn planning_duel_winner_marker_requires_arbiter_identity_without_roles() {
        let artifacts = planning_duel_artifacts(json!({
            "winner_slot": "planner_b",
            "arbiter_rationale": "Claude provided a more comprehensive diagnosis."
        }));

        let message = invalid_input_message(
            winner_artifact_from_artifacts(&artifacts, None)
                .expect_err("arbiter identity cannot be inferred without roles"),
        );

        assert!(
            message.contains(
                "planning duel winner marker requires `arbiter_family` when `planning_duel_roles` are unavailable"
            ),
            "{message}"
        );
    }

    #[test]
    fn planning_duel_winner_marker_accepts_legacy_full_payload_without_roles() {
        let artifacts = planning_duel_artifacts(json!({
            "winner_agent_cli": "claude",
            "winner_model": "claude-opus-4-7",
            "artifact_path": "planning-duel/planner_b.md",
            "arbiter_agent_cli": "gemini",
            "arbiter_model": "gemini-3.1-pro",
            "arbiter_rationale": "Claude provided a more comprehensive diagnosis."
        }));

        let winner = winner_artifact_from_artifacts(&artifacts, None)
            .expect("legacy full winner payload should still normalize");

        assert_eq!(winner.winner_family, AgentFamily::Claude);
        assert_eq!(winner.artifact_path, "planning-duel/planner_b.md");
        assert_eq!(winner.arbiter_family, AgentFamily::Gemini);
    }
}
