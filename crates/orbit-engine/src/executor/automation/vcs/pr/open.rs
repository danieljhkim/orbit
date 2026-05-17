use orbit_common::types::{ExternalRef, OrbitError, Role, Task, TaskStatus};
use orbit_tools::ToolContext;
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

use super::super::super::input::{
    canonicalize_existing_dir, input_string_field, json_number_to_string, required_input_string,
    required_job_run_id,
};
use super::super::freshness::ensure_branch_rebased_onto_base;
use super::super::git::git_output;
use super::attribution::pr_review_attribution;
use super::body::{build_batch_pr_body, default_pr_title, meaningful_execution_summary};

pub(in crate::executor::automation) fn pr_open<H: RuntimeHost + TaskHost + Sync + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    super::super::commit::commit_batch_changes(host, input)?;
    open_batch_pr(host, input)
}

pub(crate) fn open_batch_pr<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    if input.get("failed").and_then(Value::as_u64).unwrap_or(0) > 0 {
        return Err(OrbitError::Execution(
            "open_batch_pr: cannot open a batch PR while worker failures remain".to_string(),
        ));
    }

    let workspace_path_str = required_input_string(input, "workspace_path")?;
    let workspace_path = canonicalize_existing_dir(workspace_path_str, "workspace_path")?;

    let batch_id = required_job_run_id(input, "open_batch_pr")?;

    let completed_task_ids = match completed_task_ids_from_input(input) {
        Some(task_ids) => task_ids,
        None => host
            .list_tasks_filtered(None, None, None, Some(batch_id), None, None)?
            .into_iter()
            .map(|task| task.id)
            .collect(),
    };

    if completed_task_ids.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "open_batch_pr: no tasks found for job_run_id '{batch_id}'"
        )));
    }

    let mut completed_tasks = Vec::new();
    for task_id in &completed_task_ids {
        let task = host.get_task(task_id)?;
        if task.job_run_id.as_deref() != Some(batch_id) {
            return Err(OrbitError::Execution(format!(
                "open_batch_pr: task '{}' no longer belongs to batch '{}'",
                task.id, batch_id
            )));
        }
        ensure_task_can_enter_pr_review(&task)?;
        completed_tasks.push(task);
    }
    ensure_completed_tasks_have_meaningful_execution_summaries(&completed_tasks)?;

    let head = git_output(&workspace_path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let head = head.trim().to_string();
    let base = input_string_field(input, "base").unwrap_or_else(|| "main".to_string());
    let base_sync_mode = super::super::git::base_sync_mode_from_input(input)?;

    let rebase_outcome =
        ensure_branch_rebased_onto_base(&workspace_path, &head, &base, base_sync_mode)?;
    let freshness = rebase_outcome.freshness;
    let branch_was_rebased = rebase_outcome.rebased;

    let diff_output = git_output(
        &workspace_path,
        &[
            "diff",
            "--name-only",
            &format!("{}...{head}", freshness.base_ref),
        ],
    )
    .unwrap_or_default();
    let changed_files: Vec<&str> = diff_output
        .lines()
        .filter(|line| !line.is_empty())
        .collect();

    if freshness.commits_ahead == 0 {
        for task in &completed_tasks {
            let model = pr_review_attribution(host, task, batch_id)?;
            host.apply_task_automation_update(
                &task.id,
                TaskAutomationUpdate {
                    status: if task.status == TaskStatus::InProgress {
                        Some(TaskStatus::Review)
                    } else {
                        None
                    },
                    model,
                    ..TaskAutomationUpdate::default()
                },
            )?;
        }

        return Ok(json!({
            "pr_created": false,
            "reason": "no repository commits between base and head; completed tasks moved to review without a GitHub PR",
            "base": base,
            "head": head,
            "base_ref": freshness.base_ref,
            "head_ref": freshness.head_ref,
            "commits_behind": freshness.commits_behind,
            "commits_ahead": freshness.commits_ahead,
        }));
    }

    let title = input_string_field(input, "title")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default_pr_title(&completed_tasks));
    let pr_config = host.pr_config();
    let pr_opener_model = host.actor_model_identity();
    let body = input_string_field(input, "body")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            build_batch_pr_body(
                &completed_tasks,
                &freshness,
                &changed_files,
                &pr_config,
                pr_opener_model.as_deref(),
            )
        });

    let tool_context = ToolContext {
        cwd: Some(workspace_path.to_string_lossy().to_string()),
        allowed_tools: vec![],
        ..Default::default()
    };

    host.run_tool_with_context_and_role(
        "git.push",
        json!({
            "repo_root": workspace_path.to_string_lossy().to_string(),
            "branch": head,
            "force_with_lease": branch_was_rebased,
        }),
        Role::Admin,
        tool_context.clone(),
    )?;

    let pr_create = host.run_tool_with_context_and_role(
        "github.pr.create",
        json!({
            "title": title,
            "body": body,
            "base": base,
            "head": head,
        }),
        Role::Admin,
        tool_context.clone(),
    )?;
    let pr_url = pr_create
        .get("url")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            OrbitError::Execution("github.pr.create did not return a PR url".to_string())
        })?
        .to_string();
    let pr_view = host.run_tool_with_context_and_role(
        "github.pr.view",
        json!({ "pr": pr_url }),
        orbit_common::types::Role::Admin,
        tool_context,
    )?;
    let pr_number = pr_view
        .get("pull_request")
        .and_then(|value| value.get("number"))
        .and_then(json_number_to_string)
        .ok_or_else(|| {
            OrbitError::Execution("github.pr.view did not return a PR number".to_string())
        })?;

    for task in &completed_tasks {
        let model = pr_review_attribution(host, task, batch_id)?;
        host.apply_task_automation_update(
            &task.id,
            TaskAutomationUpdate {
                status: Some(TaskStatus::Review),
                external_refs: vec![ExternalRef::github_pr(pr_number.clone())?],
                model,
                ..TaskAutomationUpdate::default()
            },
        )?;
    }

    Ok(json!({
        "pr_created": true,
        "pr_number": pr_number,
        "pr_url": pr_url,
        "base": base,
        "head": head,
        "base_ref": freshness.base_ref,
        "head_ref": freshness.head_ref,
        "commits_behind": freshness.commits_behind,
        "commits_ahead": freshness.commits_ahead,
    }))
}

fn completed_task_ids_from_input(input: &Value) -> Option<Vec<String>> {
    let items = input.get("completed_task_ids")?.as_array()?;
    let ids = items
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    (!ids.is_empty()).then_some(ids)
}

fn ensure_task_can_enter_pr_review(task: &Task) -> Result<(), OrbitError> {
    if matches!(
        task.status,
        TaskStatus::InProgress | TaskStatus::Review | TaskStatus::Done
    ) {
        return Ok(());
    }

    Err(OrbitError::Execution(format!(
        "open_batch_pr: task '{}' is not promotable to review from status '{}'",
        task.id, task.status
    )))
}

fn ensure_completed_tasks_have_meaningful_execution_summaries(
    tasks: &[Task],
) -> Result<(), OrbitError> {
    for task in tasks {
        if meaningful_execution_summary(&task.execution_summary).is_none() {
            return Err(OrbitError::Execution(format!(
                "open_batch_pr: task '{}' requires a meaningful persisted execution_summary before opening the PR",
                task.id
            )));
        }
    }
    Ok(())
}
