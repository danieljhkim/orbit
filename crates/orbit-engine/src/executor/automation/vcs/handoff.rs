use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_common::types::{OrbitError, Task, TaskComment, TaskStatus};
use serde_json::Value;

use crate::context::{TaskAutomationUpdate, TaskHost};

use super::super::input::{
    canonicalize_existing_dir, input_string_field, required_input_string, required_job_run_id,
};
use super::pr::meaningful_execution_summary;

const FAILED_HANDOFF_ACTOR: &str = "system";

pub(super) struct HandoffContext {
    pub(super) batch_id: String,
    pub(super) workspace_path: PathBuf,
    pub(super) tasks: Vec<Task>,
}

pub(super) fn load_handoff_context<H: TaskHost + ?Sized>(
    host: &H,
    input: &Value,
    action: &str,
) -> Result<HandoffContext, OrbitError> {
    let workspace_path = canonicalize_existing_dir(
        required_input_string(input, "workspace_path")?,
        "workspace_path",
    )?;
    let batch_id = required_job_run_id(input, action)?.to_string();
    let task_ids = completed_task_ids_from_input(input).ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "{action}: input.completed_task_ids must contain at least one task id"
        ))
    })?;

    let mut tasks = Vec::with_capacity(task_ids.len());
    for task_id in task_ids {
        let task = host.get_task(&task_id)?;
        if task.job_run_id.as_deref() != Some(batch_id.as_str()) {
            return Err(OrbitError::Execution(format!(
                "{action}: task '{}' no longer belongs to job run '{}'",
                task.id, batch_id
            )));
        }
        if !matches!(
            task.status,
            TaskStatus::InProgress | TaskStatus::Review | TaskStatus::Done
        ) {
            return Err(OrbitError::Execution(format!(
                "{action}: task '{}' is not promotable from status '{}'",
                task.id, task.status
            )));
        }
        if meaningful_execution_summary(&task.execution_summary).is_none() {
            return Err(OrbitError::Execution(format!(
                "{action}: task '{}' requires a meaningful persisted execution_summary before PR handoff",
                task.id
            )));
        }
        tasks.push(task);
    }

    Ok(HandoffContext {
        batch_id,
        workspace_path,
        tasks,
    })
}

fn completed_task_ids_from_input(input: &Value) -> Option<Vec<String>> {
    let ids = input
        .get("completed_task_ids")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    (!ids.is_empty()).then_some(ids)
}

#[derive(Debug, Clone, Copy)]
pub(super) enum FailedHandoffPhase {
    Prepare,
    Rebase,
    Push,
    PrLookup,
    PrCreate,
    PrView,
    Promote,
    EmptyBranch,
}

impl FailedHandoffPhase {
    fn label(self) -> &'static str {
        match self {
            Self::Prepare => "prepare",
            Self::Rebase => "rebase",
            Self::Push => "push",
            Self::PrLookup => "github.pr.lookup",
            Self::PrCreate => "github.pr.create",
            Self::PrView => "github.pr.view",
            Self::Promote => "promote",
            Self::EmptyBranch => "empty-branch",
        }
    }
}

pub(super) fn record_failed_handoff<H: TaskHost + ?Sized>(
    host: &H,
    context: &HandoffContext,
    input: &Value,
    phase: FailedHandoffPhase,
    error: &OrbitError,
) -> Result<(), OrbitError> {
    let header = format!(
        "pr handoff failed [run={}] [phase={}]",
        context.batch_id,
        phase.label()
    );
    let head = input_string_field(input, "head")
        .or_else(|| current_branch(&context.workspace_path))
        .unwrap_or_else(|| "unknown".to_string());
    let base = input_string_field(input, "base").unwrap_or_else(|| "main".to_string());
    let base_ref = input_string_field(input, "base_ref")
        .map(|value| format!("Base checkpoint: {value}\n"))
        .unwrap_or_default();
    let message = format!(
        "{header}\n\nHead branch: {head}\nWorktree: {worktree}\nBase branch: {base}\n{base_ref}Failing phase: {phase}\nError: {error}\n\nRecovery:\n  Reconcile the recorded phase in this worktree, then resume the same job step. Later handoff phases have not been replayed.",
        worktree = context.workspace_path.display(),
        phase = phase.label(),
    );

    for task in &context.tasks {
        if host
            .get_task_comments(&task.id)?
            .iter()
            .any(|comment| comment.message.starts_with(&header))
        {
            continue;
        }
        host.apply_task_automation_update(
            &task.id,
            TaskAutomationUpdate {
                append_comments: vec![TaskComment {
                    at: Utc::now(),
                    by: FAILED_HANDOFF_ACTOR.to_string(),
                    message: message.clone(),
                }],
                ..TaskAutomationUpdate::default()
            },
        )?;
    }
    Ok(())
}

fn current_branch(workspace_path: &Path) -> Option<String> {
    super::git::git_output(workspace_path, &["rev-parse", "--abbrev-ref", "HEAD"])
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
