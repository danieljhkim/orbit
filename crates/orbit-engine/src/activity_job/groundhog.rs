use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use orbit_common::groundhog::{
    Attempt, Chronicle, Day, DayOutcome, FailureReport, SideEffect, SideEffectKind,
};
use orbit_common::types::activity_job::{AgentLoopSpec, GroundhogSpec};
use orbit_common::types::{
    ExecutionResult, OrbitError, TaskArtifact, TaskPlan, TaskPlanCheckpoint,
};
use orbit_tools::{GroundhogBuiltinAction, GroundhogScope, GroundhogToolHost, ToolContext};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::agent_loop_driver::drive_agent_loop_with_tool_context;
use super::audit_writer::V2AuditWriter;
use super::dispatcher::{DispatchError, DispatchOutcome, V2RuntimeHost, v2_fs_audit_logger};
use crate::WorkspaceSnapshot;

const CHRONICLE_ARTIFACT_PATH: &str = "artifacts.chronicle";
const STATE_ARTIFACT_PATH: &str = "groundhog/state.json";
const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 300_000;
const REQUIRED_GROUNDHOG_TOOLS: [&str; 3] = [
    "orbit.groundhog.checkpoint_success",
    "orbit.groundhog.checkpoint_failure",
    "orbit.groundhog.side_effect",
];

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
    let tool_ctx = host.tool_context_for_activity(fs_profile, None);
    let task = load_task(host, &tool_ctx, &task_id)?;
    let plan = parse_groundhog_plan(&task.plan, &task_id)?;
    let workspace_path = resolve_workspace_path(input, &tool_ctx, &task.workspace_path)?;
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

fn run_attempt(
    host: &dyn V2RuntimeHost,
    spec: &GroundhogSpec,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    _input: &Value,
    fs_profile: Option<&str>,
    raw_plan: &str,
    chronicle: &Chronicle,
    checkpoint: &TaskPlanCheckpoint,
    latest_failure_report: Option<&FailureReport>,
    groundhog_host: Arc<AttemptGroundhogHost>,
) -> Result<AttemptResult, DispatchError> {
    let mut tool_ctx =
        host.tool_context_for_activity(fs_profile, Some(v2_fs_audit_logger(audit.clone())));
    tool_ctx.groundhog_host = Some(groundhog_host.clone());

    let loop_input = json!({
        "prompt": build_attempt_prompt(raw_plan, chronicle, checkpoint, latest_failure_report),
    });
    let attempt_spec = build_attempt_spec(spec);
    let api_key = host.api_key_for("anthropic").ok();
    let _ = drive_agent_loop_with_tool_context(
        &attempt_spec,
        api_key.as_deref(),
        run_id,
        audit,
        &loop_input,
        tool_ctx,
    )?;

    match groundhog_host.terminal() {
        Some(TerminalVerb::Success {
            summary,
            side_effects,
        }) => Ok(AttemptResult::Success {
            summary,
            side_effects: merge_side_effects(&groundhog_host.side_effects(), &side_effects),
        }),
        Some(TerminalVerb::Failure(report)) => Ok(AttemptResult::Failure(report)),
        Some(TerminalVerb::Unsupported(reason)) => {
            Ok(AttemptResult::Failure(synthetic_failure_report(reason)))
        }
        None => Ok(AttemptResult::Failure(synthetic_failure_report(
            "attempt ended without emitting a Groundhog terminal verb",
        ))),
    }
}

fn build_attempt_spec(spec: &GroundhogSpec) -> AgentLoopSpec {
    let mut attempt_spec = spec.as_agent_loop_spec();
    attempt_spec.tools = merged_tool_allowlist(&spec.tools);
    attempt_spec.instruction = if spec.instruction.trim().is_empty() {
        groundhog_system_instruction().to_string()
    } else {
        format!(
            "{}\n\n{}",
            groundhog_system_instruction(),
            spec.instruction.trim()
        )
    };
    attempt_spec
}

fn groundhog_system_instruction() -> &'static str {
    "You are executing one Groundhog v1 checkpoint attempt. Work only on the current checkpoint, use the provided tools, and terminate the attempt by calling orbit.groundhog.checkpoint_success or orbit.groundhog.checkpoint_failure."
}

fn merged_tool_allowlist(extra_tools: &[String]) -> Vec<String> {
    let mut merged = extra_tools.to_vec();
    for required in REQUIRED_GROUNDHOG_TOOLS {
        if !merged.iter().any(|entry| entry == required) {
            merged.push(required.to_string());
        }
    }
    merged
}

fn build_attempt_prompt(
    raw_plan: &str,
    chronicle: &Chronicle,
    checkpoint: &TaskPlanCheckpoint,
    latest_failure_report: Option<&FailureReport>,
) -> String {
    let mut prompt = String::new();
    prompt.push_str("Task plan:\n");
    prompt.push_str(raw_plan.trim());
    prompt.push_str("\n\nChronicle so far (successful checkpoints only):\n");

    let mut successful = false;
    for day in &chronicle.days {
        if matches!(day.outcome, DayOutcome::Success) {
            successful = true;
            prompt.push_str(&format!(
                "- {}: {}\n",
                day.checkpoint_id,
                day.summary.trim()
            ));
        }
    }
    if !successful {
        prompt.push_str("- none yet\n");
    }

    prompt.push_str("\nCurrent checkpoint:\n");
    prompt.push_str(&format!("id: {}\n", checkpoint.id));
    prompt.push_str(&format!("spec: {}\n", checkpoint.spec));
    prompt.push_str("success_criteria:\n");
    for criterion in &checkpoint.success_criteria {
        prompt.push_str(&format!("- {:?}\n", criterion));
    }

    prompt.push_str("\nRetry context:\n");
    if let Some(report) = latest_failure_report {
        prompt.push_str(&format!(
            "what_tried: {}\nwhat_happened: {}\nnext_attempt_plan: {}\n",
            report.what_tried, report.what_happened, report.next_attempt_plan
        ));
    } else {
        prompt.push_str("none\n");
    }

    prompt.push_str(
        "\nImportant: use orbit.groundhog.checkpoint_success only when the checkpoint is complete. Use orbit.groundhog.checkpoint_failure when the attempt should end failed. Do not continue chatting after your terminal tool call.\n",
    );
    prompt
}

fn verify_checkpoint(
    host: &dyn V2RuntimeHost,
    tool_ctx: &ToolContext,
    workspace_path: &Path,
    checkpoint: &TaskPlanCheckpoint,
) -> Result<VerifierOutcome, DispatchError> {
    for criterion in &checkpoint.success_criteria {
        match criterion {
            orbit_common::types::TaskPlanSuccessCriterion::Command {
                command,
                expect_exit,
            } => {
                let result = run_workspace_command(host, tool_ctx, workspace_path, command)?;
                let actual = result.exit_code.unwrap_or(-1);
                if actual != *expect_exit {
                    let detail = truncate_for_failure_report(format!(
                        "command `{}` exited {} (expected {})\nstdout:\n{}\nstderr:\n{}",
                        command, actual, expect_exit, result.stdout, result.stderr
                    ));
                    return Ok(VerifierOutcome {
                        failure_report: Some(FailureReport {
                            what_tried: format!(
                                "verified command criterion for checkpoint `{}`",
                                checkpoint.id
                            ),
                            what_happened: detail,
                            next_attempt_plan:
                                "Fix the failing verifier condition before emitting checkpoint_success again."
                                    .to_string(),
                        }),
                    });
                }
            }
            orbit_common::types::TaskPlanSuccessCriterion::FileExists { path } => {
                let resolved = resolve_checkpoint_path(workspace_path, path);
                if !resolved.exists() {
                    return Ok(VerifierOutcome {
                        failure_report: Some(FailureReport {
                            what_tried: format!(
                                "verified file_exists criterion for checkpoint `{}`",
                                checkpoint.id
                            ),
                            what_happened: format!("required file `{}` does not exist", path),
                            next_attempt_plan:
                                "Create the expected file before emitting checkpoint_success again."
                                    .to_string(),
                        }),
                    });
                }
            }
            orbit_common::types::TaskPlanSuccessCriterion::FileContains { path, pattern } => {
                let resolved = resolve_checkpoint_path(workspace_path, path);
                let contents = fs::read_to_string(&resolved).map_err(|error| {
                    DispatchError::GroundhogFailed(format!(
                        "read verifier file {}: {error}",
                        resolved.display()
                    ))
                })?;
                if !contents.contains(pattern) {
                    return Ok(VerifierOutcome {
                        failure_report: Some(FailureReport {
                            what_tried: format!(
                                "verified file_contains criterion for checkpoint `{}`",
                                checkpoint.id
                            ),
                            what_happened: format!(
                                "file `{}` does not contain required pattern `{}`",
                                path, pattern
                            ),
                            next_attempt_plan:
                                "Update the file so the required pattern is present before emitting checkpoint_success again."
                                    .to_string(),
                        }),
                    });
                }
            }
            orbit_common::types::TaskPlanSuccessCriterion::Semantic { .. } => {}
        }
    }

    Ok(VerifierOutcome {
        failure_report: None,
    })
}

fn run_workspace_command(
    host: &dyn V2RuntimeHost,
    tool_ctx: &ToolContext,
    workspace_path: &Path,
    command: &str,
) -> Result<ExecutionResult, DispatchError> {
    let cwd = shell_single_quote(&workspace_path.display().to_string());
    let script = format!("cd {cwd} && {command}");
    let value = host
        .run_deterministic(
            "orbit_tool_call",
            &Value::Null,
            &json!({
                "tool_name": "proc.spawn",
                "args": {
                    "program": "sh",
                    "args": ["-lc", script],
                    "timeout_ms": DEFAULT_COMMAND_TIMEOUT_MS
                }
            }),
            tool_ctx.clone(),
        )
        .map_err(|error| {
            DispatchError::GroundhogFailed(format!("proc.spawn verifier call: {error}"))
        })?;
    serde_json::from_value(value).map_err(|error| {
        DispatchError::GroundhogFailed(format!("parse proc.spawn result: {error}"))
    })
}

fn resolve_checkpoint_path(workspace_path: &Path, raw: &str) -> PathBuf {
    let candidate = Path::new(raw);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace_path.join(candidate)
    }
}

fn persist_runner_artifacts(
    host: &dyn V2RuntimeHost,
    tool_ctx: &ToolContext,
    task_id: &str,
    chronicle: &Chronicle,
    state: &GroundhogRunnerState,
    status: Option<&str>,
    comment: Option<String>,
) -> Result<(), DispatchError> {
    let chronicle_json = serde_json::to_string_pretty(chronicle)
        .map_err(|error| DispatchError::GroundhogFailed(format!("serialize chronicle: {error}")))?;
    let state_json = serde_json::to_string_pretty(state)
        .map_err(|error| DispatchError::GroundhogFailed(format!("serialize state: {error}")))?;
    let artifacts = vec![
        TaskArtifact {
            path: CHRONICLE_ARTIFACT_PATH.to_string(),
            content: chronicle_json,
        },
        TaskArtifact {
            path: STATE_ARTIFACT_PATH.to_string(),
            content: state_json,
        },
    ];

    let mut args = json!({
        "id": task_id,
        "artifacts": artifacts
    });
    if let Some(status) = status {
        args["status"] = Value::String(status.to_string());
    }
    if let Some(comment) = comment {
        args["comment"] = Value::String(comment);
    }

    let _ = host
        .run_deterministic(
            "orbit_tool_call",
            &Value::Null,
            &json!({
                "tool_name": "orbit.task.update",
                "args": args
            }),
            tool_ctx.clone(),
        )
        .map_err(|error| {
            DispatchError::GroundhogFailed(format!("persist groundhog artifacts: {error}"))
        })?;
    Ok(())
}

fn load_task(
    host: &dyn V2RuntimeHost,
    tool_ctx: &ToolContext,
    task_id: &str,
) -> Result<GroundhogTaskSnapshot, DispatchError> {
    let value = host
        .run_deterministic(
            "orbit_tool_call",
            &Value::Null,
            &json!({
                "tool_name": "orbit.task.show",
                "args": { "id": task_id }
            }),
            tool_ctx.clone(),
        )
        .map_err(|error| {
            DispatchError::GroundhogFailed(format!("load task `{task_id}`: {error}"))
        })?;
    serde_json::from_value(value)
        .map_err(|error| DispatchError::GroundhogFailed(format!("parse task `{task_id}`: {error}")))
}

fn load_task_artifacts(
    host: &dyn V2RuntimeHost,
    tool_ctx: &ToolContext,
    task_id: &str,
) -> Result<Vec<TaskArtifact>, DispatchError> {
    let value = host
        .run_deterministic(
            "orbit_tool_call",
            &Value::Null,
            &json!({
                "tool_name": "orbit.task.show",
                "args": { "id": task_id, "field": "artifacts" }
            }),
            tool_ctx.clone(),
        )
        .map_err(|error| {
            DispatchError::GroundhogFailed(format!("load artifacts for task `{task_id}`: {error}"))
        })?;
    serde_json::from_value(value).map_err(|error| {
        DispatchError::GroundhogFailed(format!("parse artifacts for task `{task_id}`: {error}"))
    })
}

fn load_chronicle(
    host: &dyn V2RuntimeHost,
    tool_ctx: &ToolContext,
    task_id: &str,
) -> Result<Chronicle, DispatchError> {
    let artifacts = load_task_artifacts(host, tool_ctx, task_id)?;
    let Some(artifact) = artifacts
        .into_iter()
        .find(|artifact| artifact.path == CHRONICLE_ARTIFACT_PATH)
    else {
        return Ok(Chronicle::new(
            task_id.to_string(),
            format!("{task_id}:plan"),
        ));
    };

    serde_json::from_str(&artifact.content).map_err(|error| {
        DispatchError::GroundhogFailed(format!(
            "parse chronicle artifact for task `{task_id}`: {error}"
        ))
    })
}

fn load_state(
    host: &dyn V2RuntimeHost,
    tool_ctx: &ToolContext,
    task_id: &str,
) -> Result<GroundhogRunnerState, DispatchError> {
    let artifacts = load_task_artifacts(host, tool_ctx, task_id)?;
    let Some(artifact) = artifacts
        .into_iter()
        .find(|artifact| artifact.path == STATE_ARTIFACT_PATH)
    else {
        return Ok(GroundhogRunnerState::default());
    };

    serde_json::from_str(&artifact.content).map_err(|error| {
        DispatchError::GroundhogFailed(format!(
            "parse runner state artifact for task `{task_id}`: {error}"
        ))
    })
}

fn parse_groundhog_plan(raw_plan: &str, task_id: &str) -> Result<TaskPlan, DispatchError> {
    let label = format!("task `{task_id}` plan");
    let plan = TaskPlan::parse(raw_plan, &label)
        .map_err(|error| DispatchError::GroundhogFailed(error.to_string()))?;
    if plan.is_empty() {
        return Err(DispatchError::GroundhogFailed(format!(
            "task `{task_id}` does not contain a structured Groundhog checkpoint plan"
        )));
    }
    Ok(plan)
}

fn validate_checkpoint_alignment(
    state: &ActiveCheckpointState,
    plan: &TaskPlan,
    task_id: &str,
) -> Result<(), DispatchError> {
    let checkpoint = plan
        .checkpoints
        .get(state.checkpoint_index)
        .ok_or_else(|| {
            DispatchError::GroundhogFailed(format!(
                "task `{task_id}` current checkpoint index {} is out of bounds",
                state.checkpoint_index
            ))
        })?;
    if checkpoint.id != state.checkpoint_id {
        return Err(DispatchError::GroundhogFailed(format!(
            "task `{task_id}` checkpoint state mismatch: expected `{}`, found `{}`",
            checkpoint.id, state.checkpoint_id
        )));
    }
    Ok(())
}

fn effective_attempt_budget(checkpoint: &TaskPlanCheckpoint, fallback: u32) -> u32 {
    checkpoint.attempt_budget.max(fallback).max(1)
}

fn resolve_workspace_path(
    input: &Value,
    tool_ctx: &ToolContext,
    task_workspace_path: &Option<String>,
) -> Result<PathBuf, DispatchError> {
    let selected = input
        .get("workspace_path")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| task_workspace_path.clone())
        .or_else(|| {
            tool_ctx
                .workspace_root
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
        })
        .ok_or_else(|| {
            DispatchError::GroundhogFailed(
                "groundhog activity requires a workspace path or workspace_root".to_string(),
            )
        })?;
    Ok(PathBuf::from(selected))
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

fn merge_side_effects(recorded: &[SideEffect], reported: &[SideEffect]) -> Vec<SideEffect> {
    let mut merged = Vec::new();
    for effect in recorded.iter().chain(reported.iter()) {
        if !merged.iter().any(|existing: &SideEffect| {
            existing.kind == effect.kind
                && existing.target == effect.target
                && existing.reversible == effect.reversible
        }) {
            merged.push(effect.clone());
        }
    }
    merged
}

fn synthetic_failure_report(message: impl Into<String>) -> FailureReport {
    let message = message.into();
    FailureReport {
        what_tried: "completed a Groundhog attempt".to_string(),
        what_happened: message,
        next_attempt_plan:
            "Retry the checkpoint from a clean workspace snapshot with a narrower, more direct plan."
                .to_string(),
    }
}

fn truncate_for_failure_report(text: String) -> String {
    const MAX_LEN: usize = 4000;
    if text.len() <= MAX_LEN {
        text
    } else {
        format!("{}...[truncated]", &text[..MAX_LEN])
    }
}

fn shell_single_quote(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct GroundhogRunnerState {
    next_snapshot_n: u32,
    current: Option<ActiveCheckpointState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActiveCheckpointState {
    checkpoint_index: usize,
    checkpoint_id: String,
    started_at: DateTime<Utc>,
    attempts: Vec<Attempt>,
    latest_failure_report: Option<FailureReport>,
}

#[derive(Debug, Clone, Deserialize)]
struct GroundhogTaskSnapshot {
    plan: String,
    workspace_path: Option<String>,
}

#[derive(Debug, Clone)]
enum AttemptResult {
    Success {
        summary: String,
        side_effects: Vec<SideEffect>,
    },
    Failure(FailureReport),
}

#[derive(Debug, Clone)]
enum TerminalVerb {
    Success {
        summary: String,
        side_effects: Vec<SideEffect>,
    },
    Failure(FailureReport),
    Unsupported(String),
}

#[derive(Debug, Clone)]
struct VerifierOutcome {
    failure_report: Option<FailureReport>,
}

#[derive(Debug, Clone, Deserialize)]
struct SuccessPayload {
    summary: String,
    side_effects: Vec<SideEffect>,
}

struct AttemptGroundhogHost {
    scope: GroundhogScope,
    state: Mutex<AttemptGroundhogState>,
}

#[derive(Default)]
struct AttemptGroundhogState {
    side_effects: Vec<SideEffect>,
    terminal: Option<TerminalVerb>,
}

impl AttemptGroundhogHost {
    fn new(task_id: &str, checkpoint_id: &str) -> Self {
        Self {
            scope: GroundhogScope {
                active_day: true,
                task_id: Some(task_id.to_string()),
                checkpoint_id: Some(checkpoint_id.to_string()),
            },
            state: Mutex::new(AttemptGroundhogState::default()),
        }
    }

    fn terminal(&self) -> Option<TerminalVerb> {
        self.state
            .lock()
            .expect("groundhog attempt mutex poisoned")
            .terminal
            .clone()
    }

    fn side_effects(&self) -> Vec<SideEffect> {
        self.state
            .lock()
            .expect("groundhog attempt mutex poisoned")
            .side_effects
            .clone()
    }

    fn set_terminal(&self, terminal: TerminalVerb) -> Result<(), OrbitError> {
        let mut state = self.state.lock().expect("groundhog attempt mutex poisoned");
        if state.terminal.is_some() {
            return Err(OrbitError::Execution(
                "Groundhog attempt already recorded a terminal verb".to_string(),
            ));
        }
        state.terminal = Some(terminal);
        Ok(())
    }
}

impl GroundhogToolHost for AttemptGroundhogHost {
    fn execute(&self, action: GroundhogBuiltinAction, input: Value) -> Result<Value, OrbitError> {
        match action {
            GroundhogBuiltinAction::SideEffect => {
                let effect: SideEffect = serde_json::from_value(input).map_err(|error| {
                    OrbitError::InvalidInput(format!("parse groundhog side effect: {error}"))
                })?;
                let mut state = self.state.lock().expect("groundhog attempt mutex poisoned");
                state.side_effects.push(effect);
                Ok(json!({ "recorded": true }))
            }
            GroundhogBuiltinAction::CheckpointSuccess => {
                let payload: SuccessPayload = serde_json::from_value(input).map_err(|error| {
                    OrbitError::InvalidInput(format!("parse groundhog success payload: {error}"))
                })?;
                self.set_terminal(TerminalVerb::Success {
                    summary: payload.summary,
                    side_effects: payload.side_effects,
                })?;
                Ok(json!({ "recorded": true }))
            }
            GroundhogBuiltinAction::CheckpointFailure => {
                let report: FailureReport = serde_json::from_value(input).map_err(|error| {
                    OrbitError::InvalidInput(format!("parse groundhog failure payload: {error}"))
                })?;
                self.set_terminal(TerminalVerb::Failure(report))?;
                Ok(json!({ "recorded": true }))
            }
            GroundhogBuiltinAction::CheckpointDeviate => {
                self.set_terminal(TerminalVerb::Unsupported(
                    "checkpoint_deviate is not supported in Groundhog v1".to_string(),
                ))?;
                Ok(json!({ "recorded": true, "supported": false }))
            }
        }
    }

    fn scope(&self) -> GroundhogScope {
        self.scope.clone()
    }
}

impl Default for SuccessPayload {
    fn default() -> Self {
        Self {
            summary: String::new(),
            side_effects: Vec::new(),
        }
    }
}

#[allow(dead_code)]
fn _side_effect_kind_guard(_kind: SideEffectKind) {}
