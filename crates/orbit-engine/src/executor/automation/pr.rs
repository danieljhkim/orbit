use std::path::Path;

use orbit_store::pr_scoreboard;
use orbit_tools::ToolContext;
use orbit_types::{OrbitError, ReviewThreadStatus, Role, Task, TaskStatus};
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

use super::freshness::{ensure_branch_fresh_against_base, ensure_branch_rebased_onto_base};
use super::git::git_output;
use super::input::{
    canonicalize_existing_dir, input_string_field, json_number_to_string, required_batch_id,
    required_input_string,
};

pub(super) fn pr_open<H: RuntimeHost + TaskHost + Sync + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    super::commit::commit_batch_changes(host, input)?;
    open_batch_pr(host, input)
}

pub(super) fn git_merge<H: RuntimeHost + TaskHost + Sync + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let strategy = input
        .get("strategy")
        .and_then(Value::as_str)
        .unwrap_or("fast_forward");
    match strategy {
        "fast_forward" => super::merge_worktree::merge_batch_worktree_into_base(host, input),
        "pr_merge" => merge_batch_pr(host, input),
        other => Err(OrbitError::InvalidInput(format!(
            "git_merge: unknown strategy '{other}'; expected fast_forward or pr_merge"
        ))),
    }
}

pub(super) fn merge_batch_pr<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let batch_id = required_batch_id(input, "merge_batch_pr")?;

    let batch_tasks = host.list_tasks_filtered(None, None, None, Some(batch_id))?;
    if batch_tasks.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "merge_batch_pr: no tasks found for batch_id '{batch_id}'"
        )));
    }

    // Find pr_number from the first task that has one
    let pr_number = batch_tasks
        .iter()
        .find_map(|t| t.pr_number.as_deref())
        .ok_or_else(|| {
            OrbitError::InvalidInput("merge_batch_pr: no task in batch has a pr_number".to_string())
        })?
        .to_string();

    let workspace_path = resolve_batch_workspace_path(host, input, batch_id)?;

    // Get the current branch from the workspace
    let head = git_output(&workspace_path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let head = head.trim().to_string();
    let base = input_string_field(input, "base").unwrap_or_else(|| "main".to_string());

    // Check that ALL tasks have APPROVED pr_status
    for task in &batch_tasks {
        let pr_status_raw = task.pr_status.as_deref().unwrap_or("none");
        let review_decision = super::review::normalize_review_decision(pr_status_raw);
        if review_decision != "APPROVED" {
            return Err(OrbitError::Execution(format!(
                "task '{}' is not approved (pr_status={pr_status_raw})",
                task.id
            )));
        }
    }

    // Check that ALL tasks are in Review or Done status
    for task in &batch_tasks {
        if !matches!(task.status, TaskStatus::Review | TaskStatus::Done) {
            return Err(OrbitError::Execution(format!(
                "task '{}' must be in Review or Done before merge_batch_pr; current status is {}",
                task.id, task.status
            )));
        }
    }

    ensure_branch_fresh_against_base(&workspace_path, &head, &base)?;

    let tool_context = ToolContext {
        cwd: Some(workspace_path.to_string_lossy().to_string()),
        allowed_tools: vec![],
        ..Default::default()
    };
    host.run_tool_with_context_and_role(
        "github.pr.merge",
        json!({
            "pr": pr_number,
            "strategy": "squash",
            // Do not pass --delete-branch to `gh pr merge` because the local
            // branch is still attached to the shared worktree and `gh` would
            // fail trying to delete it.  We delete the remote branch separately
            // below, tolerating errors (the repo may auto-delete branches after
            // merge).
            "delete_branch": false,
        }),
        Role::Admin,
        tool_context,
    )?;

    // Best-effort remote branch cleanup.  Some repos have GitHub's
    // "Automatically delete head branches" enabled, so the remote ref may
    // already be gone — ignore errors.
    let _ =
        super::git::git_command_success(&workspace_path, &["push", "origin", "--delete", &head]);

    let batch_requires_revision = batch_tasks.iter().any(task_required_revision);
    let batch_author = batch_tasks.iter().find_map(|task| {
        task.implemented_by
            .clone()
            .or_else(|| task.created_by.clone())
    });

    // Advance ALL batch tasks to Done status
    for task in &batch_tasks {
        host.apply_task_automation_update(
            &task.id,
            TaskAutomationUpdate {
                status: if task.status == TaskStatus::Review {
                    Some(TaskStatus::Done)
                } else {
                    None
                },
                pr_number: Some(pr_number.clone()),
                ..TaskAutomationUpdate::default()
            },
        )?;
    }

    if host.scoring_enabled()
        && let Some(model) = batch_author
    {
        let _ = if batch_requires_revision {
            pr_scoreboard::record_pr_count_with_revision(host.scoreboard_dir(), &model)
        } else {
            pr_scoreboard::record_pr_count_without_revision(host.scoreboard_dir(), &model)
        };
    }

    Ok(json!({ "merged": true }))
}

pub(super) fn open_batch_pr<H: RuntimeHost + TaskHost + ?Sized>(
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

    let batch_id = required_batch_id(input, "open_batch_pr")?;

    let completed_task_ids = match completed_task_ids_from_input(input) {
        Some(task_ids) => task_ids,
        None => host
            .list_tasks_filtered(None, None, None, Some(batch_id))?
            .into_iter()
            .map(|task| task.id)
            .collect(),
    };

    if completed_task_ids.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "open_batch_pr: no tasks found for batch_id '{batch_id}'"
        )));
    }

    let head = git_output(&workspace_path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let head = head.trim().to_string();
    let base = input_string_field(input, "base").unwrap_or_else(|| "main".to_string());

    let rebase_outcome = ensure_branch_rebased_onto_base(&workspace_path, &head, &base)?;
    let freshness = rebase_outcome.freshness;
    let branch_was_rebased = rebase_outcome.rebased;

    let diff_output = git_output(
        &workspace_path,
        &["diff", "--name-only", &format!("{base}...{head}")],
    )
    .unwrap_or_default();
    let changed_files: Vec<&str> = diff_output
        .lines()
        .filter(|line| !line.is_empty())
        .collect();

    let mut completed_tasks = Vec::new();
    for task_id in &completed_task_ids {
        let task = host.get_task(task_id)?;
        if task.batch_id.as_deref() != Some(batch_id) {
            return Err(OrbitError::Execution(format!(
                "open_batch_pr: task '{}' no longer belongs to batch '{}'",
                task.id, batch_id
            )));
        }
        ensure_task_can_enter_pr_review(&task)?;
        completed_tasks.push(task);
    }
    let id_labels: Vec<&str> = completed_tasks
        .iter()
        .map(|task| task.id.as_str())
        .collect();
    let ids_joined = id_labels.join(", ");

    let title = format!("feat: parallel batch [{ids_joined}]");
    let body = build_batch_pr_body(&completed_tasks, &freshness, &changed_files);

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

fn resolve_batch_workspace_path<H: RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
    batch_id: &str,
) -> Result<std::path::PathBuf, OrbitError> {
    match input_string_field(input, "workspace_path") {
        Some(path) => canonicalize_existing_dir(&path, "workspace_path"),
        None => {
            let repo_root = host.repo_root()?;
            super::parallel::resolve_shared_worktree_path(Path::new(&repo_root), batch_id)
        }
    }
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

fn build_batch_pr_body(
    tasks: &[Task],
    freshness: &super::freshness::BranchFreshness,
    changed_files: &[&str],
) -> String {
    let task_sections = tasks
        .iter()
        .map(render_task_section)
        .collect::<Vec<_>>()
        .join("\n\n");
    let changed_files_section = changed_files
        .iter()
        .map(|file| format!("- `{file}`"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut body = format!(
        "## Tasks\n{}\n\n## Branch Freshness\n- Base ref: `{}`\n- Head ref: `{}`\n- Behind base: {}\n- Ahead of base: {}\n\n## Files Changed\n{}",
        task_sections,
        freshness.base_ref,
        freshness.head_ref,
        freshness.commits_behind,
        freshness.commits_ahead,
        changed_files_section
    );

    if let Some(signature) = batch_pr_signature(tasks) {
        body.push_str("\n\n");
        body.push_str(&signature);
    }

    body
}

fn render_task_section(task: &Task) -> String {
    let mut section = format!("### {}: {}", task.id, task.title.trim());
    let summary = task.execution_summary.trim();
    if !summary.is_empty() {
        section.push_str(&format!(
            "\n<details><summary>Execution Summary</summary>\n\n{}\n\n</details>",
            summary
        ));
    }
    section
}

fn batch_pr_signature(tasks: &[Task]) -> Option<String> {
    tasks.iter().find_map(|task| {
        let model = task
            .implemented_by
            .as_deref()
            .or(task.created_by.as_deref())?;
        Some(format!("*authored by: {model}*"))
    })
}

fn task_required_revision(task: &Task) -> bool {
    task.history.iter().any(|entry| {
        entry.event == "status_changed"
            && entry.from_status == Some(TaskStatus::Review)
            && matches!(
                entry.to_status,
                Some(TaskStatus::Backlog | TaskStatus::InProgress | TaskStatus::Rejected)
            )
    }) || task
        .review_threads
        .iter()
        .any(|thread| thread.status == ReviewThreadStatus::Resolved)
}
