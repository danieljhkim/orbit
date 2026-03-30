use orbit_store::pr_scoreboard;
use orbit_tools::ToolContext;
use orbit_types::{OrbitError, Role, TaskStatus};
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

use super::freshness::ensure_branch_fresh_against_base;
use super::git::git_output;
use super::input::{
    canonicalize_existing_dir, input_string_field, json_number_to_string, required_input_string,
};

pub(super) fn merge_pr_from_task<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let task = host.get_task(task_id)?;

    if task.pr_number.is_none() {
        return Ok(json!({ "merged": false, "skipped": true }));
    }

    let repo_root = canonicalize_existing_dir(
        task.repo_root
            .as_deref()
            .or(task.workspace_path.as_deref())
            .ok_or_else(|| {
                OrbitError::InvalidInput(
                    "merge_pr_from_task requires task.repo_root or task.workspace_path".to_string(),
                )
            })?,
        "repo_root",
    )?;
    let pr_number = task.pr_number.as_deref().ok_or_else(|| {
        OrbitError::InvalidInput("merge_pr_from_task requires task.pr_number".to_string())
    })?;
    let head = format!("orbit/{task_id}");
    let base = input_string_field(input, "base").unwrap_or_else(|| "agent-main".to_string());
    let pr_status_raw = task.pr_status.as_deref().unwrap_or("none");
    let review_decision = super::review::normalize_review_decision(pr_status_raw);
    if review_decision != "APPROVED" {
        return Err(OrbitError::Execution(format!(
            "pull request '{pr_number}' is not approved (pr_status={pr_status_raw})"
        )));
    }

    if !matches!(task.status, TaskStatus::Review | TaskStatus::Done) {
        return Err(OrbitError::Execution(format!(
            "task '{}' must be in review before merge_pr_from_task; current status is {}",
            task.id, task.status
        )));
    }
    ensure_branch_fresh_against_base(&repo_root, &head, &base)?;

    let tool_context = ToolContext {
        cwd: Some(repo_root.to_string_lossy().to_string()),
        allowed_tools: vec![],
        ..Default::default()
    };
    host.run_tool_with_context_and_role(
        "github.pr.merge",
        json!({
            "pr": pr_number,
            "strategy": "squash",
        }),
        Role::Admin,
        tool_context,
    )?;

    host.apply_task_automation_update(
        task_id,
        TaskAutomationUpdate {
            status: if task.status == TaskStatus::Review {
                Some(TaskStatus::Done)
            } else {
                None
            },
            pr_number: Some(pr_number.to_string()),
            ..TaskAutomationUpdate::default()
        },
    )?;

    if host.scoring_enabled()
        && let (Some(agent), Some(model)) = (
            task.actor_identity.agent_name(),
            task.actor_identity.agent_model(),
        )
    {
        let _ = pr_scoreboard::record_pr_merged(host.scoreboard_dir(), agent, model);
    }

    Ok(json!({
        "merged": true,
    }))
}

pub(super) fn open_pr_from_task<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let task = host.get_task(task_id)?;
    let repo_root = canonicalize_existing_dir(
        task.repo_root
            .as_deref()
            .or(task.workspace_path.as_deref())
            .ok_or_else(|| {
                OrbitError::InvalidInput(
                    "open_pr_from_task requires task.repo_root or task.workspace_path".to_string(),
                )
            })?,
        "repo_root",
    )?;
    let head = format!("orbit/{task_id}");
    let base = input_string_field(input, "base").unwrap_or_else(|| "agent-main".to_string());

    // Idempotent: if the task already has a PR number, skip creation and return
    // the existing PR info.  This handles the case where the agent (or a
    // previous run) already opened the PR.
    if let Some(ref existing_pr) = task.pr_number {
        let tool_context = ToolContext {
            cwd: Some(repo_root.to_string_lossy().to_string()),
            allowed_tools: vec![],
            ..Default::default()
        };
        let pr_view = host.run_tool_with_context_and_role(
            "github.pr.view",
            json!({ "pr": existing_pr }),
            Role::Admin,
            tool_context,
        );
        if pr_view.is_ok() {
            return Ok(json!({}));
        }
        // If the PR view failed (e.g. PR was closed/deleted), fall through to
        // create a new one.
    }

    let freshness = ensure_branch_fresh_against_base(&repo_root, &head, &base)?;

    // Derive changed files from git diff against base.
    let diff_output = git_output(
        &repo_root,
        &["diff", "--name-only", &format!("{base}...{head}")],
    )
    .unwrap_or_default();
    let changed_files: Vec<&str> = diff_output
        .lines()
        .filter(|line| !line.is_empty())
        .collect();

    let body = format!(
        "## Changes\n{}\n\n## Branch Freshness\n- Base ref: `{}`\n- Head ref: `{}`\n- Behind base: {}\n- Ahead of base: {}\n\n## Files Changed\n{}",
        task.execution_summary.trim(),
        freshness.base_ref,
        freshness.head_ref,
        freshness.commits_behind,
        freshness.commits_ahead,
        changed_files
            .iter()
            .map(|f| format!("- `{f}`"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let title = task.title.trim().to_string();
    let tool_context = ToolContext {
        cwd: Some(repo_root.to_string_lossy().to_string()),
        allowed_tools: vec![],
        agent_name: task.actor_identity.agent_name().map(ToOwned::to_owned),
        model_name: task.actor_identity.agent_model().map(ToOwned::to_owned),
        ..Default::default()
    };

    // Push the branch so GitHub can see it before creating the PR.
    host.run_tool_with_context_and_role(
        "git.push",
        json!({
            "repo_root": repo_root.to_string_lossy().to_string(),
            "branch": head,
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
            "label": "orbit",
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
        Role::Admin,
        tool_context,
    )?;
    let pr_number = pr_view
        .get("pull_request")
        .and_then(|value| value.get("number"))
        .and_then(json_number_to_string)
        .ok_or_else(|| {
            OrbitError::Execution("github.pr.view did not return a PR number".to_string())
        })?;

    let target_status = if task.status == TaskStatus::InProgress {
        Some(TaskStatus::Review)
    } else {
        None
    };
    host.apply_task_automation_update(
        task_id,
        TaskAutomationUpdate {
            status: target_status,
            pr_number: Some(pr_number.clone()),
            execution_summary: Some(body.clone()),
            ..TaskAutomationUpdate::default()
        },
    )?;

    Ok(json!({}))
}

pub(super) fn open_batch_pr<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let workspace_path_str = required_input_string(input, "workspace_path")?;
    let workspace_path = canonicalize_existing_dir(workspace_path_str, "workspace_path")?;

    let batch_id = input
        .get("run_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            OrbitError::InvalidInput("open_batch_pr requires input.run_id".to_string())
        })?;

    let batch_tasks = host.list_tasks_filtered(None, None, None, Some(batch_id))?;
    let completed_task_ids: Vec<String> = batch_tasks.iter().map(|t| t.id.clone()).collect();

    if completed_task_ids.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "open_batch_pr: no tasks found for batch_id '{batch_id}'"
        )));
    }

    let head = git_output(&workspace_path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let head = head.trim().to_string();
    let base = input_string_field(input, "base").unwrap_or_else(|| "agent-main".to_string());

    let freshness = ensure_branch_fresh_against_base(&workspace_path, &head, &base)?;

    let diff_output = git_output(
        &workspace_path,
        &["diff", "--name-only", &format!("{base}...{head}")],
    )
    .unwrap_or_default();
    let changed_files: Vec<&str> = diff_output
        .lines()
        .filter(|line| !line.is_empty())
        .collect();

    let mut task_lines = Vec::new();
    let mut id_labels = Vec::new();
    for task_id in &completed_task_ids {
        let task = host.get_task(task_id)?;
        task_lines.push(format!("- {}: {}", task_id, task.title.trim()));
        id_labels.push(task_id.clone());
    }
    let ids_joined = id_labels.join(", ");

    let title = format!("feat: parallel batch [{ids_joined}]");
    let body = format!(
        "## Tasks\n{}\n\n## Branch Freshness\n- Base ref: `{}`\n- Head ref: `{}`\n- Behind base: {}\n- Ahead of base: {}\n\n## Files Changed\n{}",
        task_lines.join("\n"),
        freshness.base_ref,
        freshness.head_ref,
        freshness.commits_behind,
        freshness.commits_ahead,
        changed_files
            .iter()
            .map(|f| format!("- `{f}`"))
            .collect::<Vec<_>>()
            .join("\n")
    );

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
            "label": "orbit",
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
        Role::Admin,
        tool_context,
    )?;
    let pr_number = pr_view
        .get("pull_request")
        .and_then(|value| value.get("number"))
        .and_then(json_number_to_string)
        .ok_or_else(|| {
            OrbitError::Execution("github.pr.view did not return a PR number".to_string())
        })?;

    for task_id in &completed_task_ids {
        host.apply_task_automation_update(
            task_id,
            TaskAutomationUpdate {
                status: Some(TaskStatus::Review),
                pr_number: Some(pr_number.clone()),
                ..TaskAutomationUpdate::default()
            },
        )?;
    }

    Ok(json!({}))
}
