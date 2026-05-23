mod attempt;
mod persistence;
mod verifier;

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use orbit_common::groundhog::{Attempt, Chronicle, Day, DayOutcome};
use orbit_common::types::activity_job::GroundhogSpec;
use orbit_tools::ToolContext;
use serde_json::{Value, json};

use self::attempt::{AttemptGroundhogHost, AttemptResult, run_attempt};
use self::persistence::{
    ActiveCheckpointState, effective_attempt_budget, load_chronicle, load_state, load_task,
    parse_groundhog_plan, persist_runner_artifacts, validate_checkpoint_alignment,
};
use self::verifier::verify_checkpoint;
use super::audit_writer::V2AuditWriter;
use super::dispatcher::{DispatchError, DispatchOutcome, V2RuntimeHost};
use super::workspace::resolve_subprocess_cwd;
use crate::WorkspaceSnapshot;

pub fn run_groundhog_activity(
    host: &dyn V2RuntimeHost,
    _activity_name: &str,
    spec: &GroundhogSpec,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    input: &Value,
    fs_profile: Option<&str>,
) -> Result<DispatchOutcome, DispatchError> {
    let task_id = required_input_string(input, "task_id")?;
    let tool_ctx = host.tool_context_for_activity(
        Some(run_id),
        fs_profile,
        None,
        spec.proc_allowed_programs.as_deref(),
    );
    let task = load_task(host, &tool_ctx, &task_id)?;
    let plan = parse_groundhog_plan(&task.plan, &task_id)?;
    let workspace_path = resolve_workspace_path(input, &tool_ctx, &None)?;
    let mut chronicle = load_chronicle(host, &tool_ctx, &task_id)?;
    let mut state = load_state(host, &tool_ctx, &task_id)?;

    if chronicle.task_id.is_empty() {
        chronicle = Chronicle::new(task_id.clone(), format!("{task_id}:plan"));
    }
    if state.next_snapshot_n == 0 {
        state.next_snapshot_n = chronicle.days.len() as u32 + 1;
    }

    loop {
        if let Some(current) = &state.current {
            validate_checkpoint_alignment(current, &plan, &task_id)?;
        }

        if state.current.is_none() && chronicle.days.len() >= plan.checkpoints.len() {
            persist_runner_artifacts(host, &tool_ctx, &task_id, &chronicle, &state, None, None)?;
            return Ok(DispatchOutcome {
                success: true,
                output: json!({
                    "task_id": task_id,
                    "status": "success",
                    "completed_checkpoints": chronicle.days.len(),
                    "chronicle_days": chronicle.days.len(),
                }),
                message: Some("groundhog run completed all checkpoints".to_string()),
                invocation: None,
            });
        }

        let checkpoint_index = state
            .current
            .as_ref()
            .map(|current| current.checkpoint_index)
            .unwrap_or(chronicle.days.len());
        let checkpoint = plan.checkpoints.get(checkpoint_index).ok_or_else(|| {
            DispatchError::GroundhogFailed(format!(
                "checkpoint index {checkpoint_index} is out of bounds for task `{task_id}`"
            ))
        })?;
        let attempt_budget = effective_attempt_budget(checkpoint, spec.attempt_budget_default);

        let mut checkpoint_state = state
            .current
            .take()
            .unwrap_or_else(|| ActiveCheckpointState {
                checkpoint_index,
                checkpoint_id: checkpoint.id.clone(),
                started_at: Utc::now(),
                attempts: Vec::new(),
                latest_failure_report: None,
            });

        let snapshot = WorkspaceSnapshot::create(&task_id, state.next_snapshot_n, &workspace_path)
            .map_err(|error| DispatchError::GroundhogFailed(error.to_string()))?;
        state.next_snapshot_n += 1;

        let attempt_started_at = Utc::now();
        let attempt_host = Arc::new(AttemptGroundhogHost::new(&task_id, &checkpoint.id));
        let attempt_result = run_attempt(
            host,
            spec,
            run_id,
            audit.clone(),
            input,
            fs_profile,
            task.plan.as_str(),
            &chronicle,
            checkpoint,
            checkpoint_state.latest_failure_report.as_ref(),
            attempt_host.clone(),
        )?;
        let attempt_ended_at = Utc::now();

        match attempt_result {
            AttemptResult::Success {
                summary,
                side_effects,
            } => {
                let verifier = verify_checkpoint(host, &tool_ctx, &workspace_path, checkpoint)?;
                if let Some(failure_report) = verifier.failure_report {
                    WorkspaceSnapshot::rewind(&snapshot)
                        .map_err(|error| DispatchError::GroundhogFailed(error.to_string()))?;
                    checkpoint_state.attempts.push(Attempt {
                        started_at: attempt_started_at,
                        ended_at: attempt_ended_at,
                        tool_calls: Vec::new(),
                        failure_report: Some(failure_report.clone()),
                        workspace_reverted: true,
                    });
                    checkpoint_state.latest_failure_report = Some(failure_report.clone());
                    if checkpoint_state.attempts.len() as u32 >= attempt_budget {
                        let day = Day {
                            checkpoint_id: checkpoint.id.clone(),
                            attempts: checkpoint_state.attempts.clone(),
                            outcome: DayOutcome::Abandoned {
                                reason: "attempt budget exhausted".to_string(),
                            },
                            summary: "checkpoint abandoned after verifier failures".to_string(),
                            side_effects: Vec::new(),
                            started_at: checkpoint_state.started_at,
                            ended_at: Utc::now(),
                        };
                        chronicle.days.push(day);
                        state.current = None;
                        persist_runner_artifacts(
                            host,
                            &tool_ctx,
                            &task_id,
                            &chronicle,
                            &state,
                            Some("blocked"),
                            Some(format!(
                                "Groundhog blocked task after `{}` exhausted its attempt budget.",
                                checkpoint.id
                            )),
                        )?;
                        return Ok(DispatchOutcome {
                            success: false,
                            output: json!({
                                "task_id": task_id,
                                "status": "blocked",
                                "checkpoint_id": checkpoint.id,
                                "reason": "attempt budget exhausted",
                            }),
                            message: Some(format!(
                                "checkpoint `{}` exhausted its attempt budget",
                                checkpoint.id
                            )),
                            invocation: None,
                        });
                    }
                    state.current = Some(checkpoint_state);
                    persist_runner_artifacts(
                        host, &tool_ctx, &task_id, &chronicle, &state, None, None,
                    )?;
                    continue;
                }

                WorkspaceSnapshot::commit_success(&snapshot, &summary)
                    .map_err(|error| DispatchError::GroundhogFailed(error.to_string()))?;
                checkpoint_state.attempts.push(Attempt {
                    started_at: attempt_started_at,
                    ended_at: attempt_ended_at,
                    tool_calls: Vec::new(),
                    failure_report: None,
                    workspace_reverted: false,
                });
                chronicle.days.push(Day {
                    checkpoint_id: checkpoint.id.clone(),
                    attempts: checkpoint_state.attempts.clone(),
                    outcome: DayOutcome::Success,
                    summary,
                    side_effects,
                    started_at: checkpoint_state.started_at,
                    ended_at: Utc::now(),
                });
                state.current = None;
                persist_runner_artifacts(
                    host, &tool_ctx, &task_id, &chronicle, &state, None, None,
                )?;
            }
            AttemptResult::Failure(failure_report) => {
                WorkspaceSnapshot::rewind(&snapshot)
                    .map_err(|error| DispatchError::GroundhogFailed(error.to_string()))?;
                checkpoint_state.attempts.push(Attempt {
                    started_at: attempt_started_at,
                    ended_at: attempt_ended_at,
                    tool_calls: Vec::new(),
                    failure_report: Some(failure_report.clone()),
                    workspace_reverted: true,
                });
                checkpoint_state.latest_failure_report = Some(failure_report);
                if checkpoint_state.attempts.len() as u32 >= attempt_budget {
                    chronicle.days.push(Day {
                        checkpoint_id: checkpoint.id.clone(),
                        attempts: checkpoint_state.attempts.clone(),
                        outcome: DayOutcome::Abandoned {
                            reason: "attempt budget exhausted".to_string(),
                        },
                        summary: "checkpoint abandoned after repeated failures".to_string(),
                        side_effects: Vec::new(),
                        started_at: checkpoint_state.started_at,
                        ended_at: Utc::now(),
                    });
                    state.current = None;
                    persist_runner_artifacts(
                        host,
                        &tool_ctx,
                        &task_id,
                        &chronicle,
                        &state,
                        Some("blocked"),
                        Some(format!(
                            "Groundhog blocked task after `{}` exhausted its attempt budget.",
                            checkpoint.id
                        )),
                    )?;
                    return Ok(DispatchOutcome {
                        success: false,
                        output: json!({
                            "task_id": task_id,
                            "status": "blocked",
                            "checkpoint_id": checkpoint.id,
                            "reason": "attempt budget exhausted",
                        }),
                        message: Some(format!(
                            "checkpoint `{}` exhausted its attempt budget",
                            checkpoint.id
                        )),
                        invocation: None,
                    });
                }
                state.current = Some(checkpoint_state);
                persist_runner_artifacts(
                    host, &tool_ctx, &task_id, &chronicle, &state, None, None,
                )?;
            }
        }
    }
}

fn resolve_workspace_path(
    input: &Value,
    tool_ctx: &ToolContext,
    task_workspace_path: &Option<String>,
) -> Result<PathBuf, DispatchError> {
    let task_ctx = task_workspace_path
        .as_ref()
        .map(|workspace_path| json!({ "workspace_path": workspace_path }));
    resolve_subprocess_cwd(input, task_ctx.as_ref(), tool_ctx.workspace_root.as_deref())?
        .ok_or_else(|| {
            DispatchError::GroundhogFailed(
                "groundhog activity requires a workspace path or workspace_root".to_string(),
            )
        })
}

fn required_input_string(input: &Value, key: &str) -> Result<String, DispatchError> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| {
            DispatchError::GroundhogFailed(format!("missing `{key}` in groundhog input"))
        })
}

#[cfg(test)]
mod tests;
