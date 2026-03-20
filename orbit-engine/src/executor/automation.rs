use std::path::{Path, PathBuf};

use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_tools::ToolContext;
use orbit_types::{Activity, JobRunState, OrbitError, Role, TaskStatus, TaskType};
use serde::Deserialize;
use serde_json::{Value, json};

use super::ActivityExecutor;
use crate::activity_runner::validate_activity_output_schema;
use crate::context::{
    ACTIVITY_EXECUTION_FAILED, AttemptOutcome, EngineHost, ExecutionContext, RuntimeHost,
    TaskAutomationUpdate, TaskHost,
};

const AUTOMATION_CREATE_TASK_WORKTREE: &str = "create_task_worktree";
const AUTOMATION_START_TASK: &str = "start_task";
const AUTOMATION_UPDATE_TASK: &str = "update_task";
const AUTOMATION_COMMIT_TASK_CHANGES: &str = "commit_task_changes";
const AUTOMATION_MERGE_PR_FROM_TASK: &str = "merge_pr_from_task";
const AUTOMATION_OPEN_PR_FROM_TASK: &str = "open_pr_from_task";
const AUTOMATION_FINALIZE_TASK_WORKTREE: &str = "finalize_task_worktree";

#[derive(Debug, Clone, Deserialize)]
struct AutomationSpec {
    action: String,
}

#[derive(Debug, Clone)]
struct BranchFreshness {
    base_ref: String,
    head_ref: String,
    commits_behind: u64,
    commits_ahead: u64,
}

pub struct AutomationExecutor;

impl ActivityExecutor for AutomationExecutor {
    fn spec_type(&self) -> &str {
        "automation"
    }

    fn execute(&self, host: &dyn EngineHost, execution: &ExecutionContext) -> AttemptOutcome {
        match execute(host, &execution.activity, &execution.input) {
            Ok(result) => {
                if let Err(err) = validate_activity_output_schema(&execution.activity, &result) {
                    return AttemptOutcome {
                        state: JobRunState::Failed,
                        exit_code: Some(0),
                        duration_ms: None,
                        response_json: Some(result),
                        error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
                        error_message: Some(err.to_string()),
                        protocol_violation: false,
                    };
                }
                AttemptOutcome {
                    state: JobRunState::Success,
                    exit_code: Some(0),
                    duration_ms: None,
                    response_json: Some(result),
                    error_code: None,
                    error_message: None,
                    protocol_violation: false,
                }
            }
            Err(err) => AttemptOutcome {
                state: JobRunState::Failed,
                exit_code: Some(1),
                duration_ms: None,
                response_json: None,
                error_code: Some(ACTIVITY_EXECUTION_FAILED.to_string()),
                error_message: Some(err.to_string()),
                protocol_violation: false,
            },
        }
    }
}

pub fn execute<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    activity: &Activity,
    input: &Value,
) -> Result<Value, OrbitError> {
    let spec: AutomationSpec =
        serde_json::from_value(activity.spec_config.clone()).map_err(|error| {
            OrbitError::InvalidInput(format!("invalid automation spec_config: {error}"))
        })?;

    match spec.action.as_str() {
        AUTOMATION_CREATE_TASK_WORKTREE => create_task_worktree(host, input),
        AUTOMATION_START_TASK => start_task(host, input),
        AUTOMATION_UPDATE_TASK => update_task(host, input),
        AUTOMATION_COMMIT_TASK_CHANGES => commit_task_changes(host, input),
        AUTOMATION_MERGE_PR_FROM_TASK => merge_pr_from_task(host, input),
        AUTOMATION_OPEN_PR_FROM_TASK => open_pr_from_task(host, input),
        AUTOMATION_FINALIZE_TASK_WORKTREE => finalize_task_worktree(input),
        other => Err(OrbitError::InvalidInput(format!(
            "unsupported automation action '{other}'"
        ))),
    }
}

fn create_task_worktree<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let repo_root = host.repo_root().or_else(|_| {
        let task = host.get_task(task_id)?;
        task.workspace_path.ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "task '{task_id}' must define workspace_path when Orbit cannot derive the repository root automatically"
            ))
        })
    })?;
    let repo_root = canonicalize_existing_dir(&repo_root, "repo_root")?;
    let base = input_string_field(input, "base").unwrap_or_else(|| "agent-main".to_string());
    let branch = format!("orbit/{task_id}");
    let worktree_path = resolve_task_worktree_path(&repo_root, task_id)?;

    if worktree_path.exists() {
        ensure_existing_task_worktree(&worktree_path, &branch)?;
    } else {
        fetch_remote_base(&repo_root, &base);
        let start_point = resolve_worktree_start_point(&repo_root, &base)?;
        create_or_attach_task_worktree(&repo_root, &worktree_path, &branch, &start_point)?;
    }

    let canonical_worktree = worktree_path.canonicalize().map_err(|error| {
        OrbitError::Execution(format!(
            "failed to canonicalize task worktree '{}': {error}",
            worktree_path.display()
        ))
    })?;
    let canonical_repo_root = repo_root.canonicalize().map_err(|error| {
        OrbitError::Execution(format!(
            "failed to canonicalize repo_root '{}': {error}",
            repo_root.display()
        ))
    })?;

    host.apply_task_automation_update(
        task_id,
        TaskAutomationUpdate {
            workspace_path: Some(canonical_worktree.to_string_lossy().to_string()),
            repo_root: Some(canonical_repo_root.to_string_lossy().to_string()),
            branch: Some(branch.clone()),
            ..TaskAutomationUpdate::default()
        },
    )?;

    Ok(json!({
        "workspace_path": canonical_worktree.to_string_lossy().to_string(),
        "repo_root": canonical_repo_root.to_string_lossy().to_string(),
        "branch": branch,
    }))
}

fn start_task<H: TaskHost + ?Sized>(host: &H, input: &Value) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let task = host.start_task(
        task_id,
        input_string_field(input, "note"),
        input_string_field(input, "comment"),
    )?;
    Ok(json!({
        "task_id": task.id.to_string(),
        "status": task.status,
        "title": task.title,
        "description": task.description,
        "plan": task.plan,
        "context_files": task.context_files,
    }))
}

fn update_task<H: TaskHost + ?Sized>(host: &H, input: &Value) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let status = required_input_string(input, "status")?
        .parse::<TaskStatus>()
        .map_err(|error| OrbitError::InvalidInput(format!("invalid input.status: {error}")))?;
    let task = host.update_task_from_activity(
        task_id,
        status,
        input_string_field(input, "execution_summary"),
        input_string_array_field(input, "files_changed")?,
        input_string_field(input, "comment"),
        input_string_field(input, "note"),
    )?;
    serde_json::to_value(task).map_err(|error| {
        OrbitError::Execution(format!("failed to serialize updated task: {error}"))
    })
}

fn commit_task_changes<H: TaskHost + ?Sized>(host: &H, input: &Value) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let task = host.get_task(task_id)?;
    let workspace_path = canonicalize_existing_dir(
        &input_workspace_path(input)
            .or_else(|| task.workspace_path.clone())
            .ok_or_else(|| {
                OrbitError::InvalidInput(
                    "commit_task_changes requires input.workspace_path or task.workspace_path"
                        .to_string(),
                )
            })?,
        "workspace_path",
    )?;
    let repo_root = canonicalize_existing_dir(
        &input_string_field(input, "repo_root")
            .or_else(|| input_workspace_path(input))
            .or_else(|| task.repo_root.clone())
            .ok_or_else(|| {
                OrbitError::InvalidInput(
                    "commit_task_changes requires input.repo_root, input.workspace_path, or task.repo_root"
                        .to_string(),
                )
            })?,
        "repo_root",
    )?;
    let expected_branch =
        input_string_field(input, "branch").unwrap_or_else(|| format!("orbit/{task_id}"));
    let summary =
        input_string_field(input, "summary").unwrap_or_else(|| task.execution_summary.clone());
    if summary.trim().is_empty() {
        return Err(OrbitError::Execution(format!(
            "task '{}' commit_task_changes requires a non-empty summary from input.summary or task.execution_summary",
            task_id
        )));
    }

    let actual_branch = git_output(&workspace_path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if actual_branch.trim() != expected_branch {
        return Err(OrbitError::Execution(format!(
            "workspace '{}' is on branch '{}' but '{}' was expected",
            workspace_path.display(),
            actual_branch.trim(),
            expected_branch
        )));
    }

    ensure_no_unmerged_changes(&workspace_path)?;
    git_success(&workspace_path, &["add", "--all", "--", "."])?;
    let changed_files = git_output_paths(
        &workspace_path,
        &["diff", "--cached", "--name-only", "-z", "--relative"],
    )?;
    if changed_files.is_empty() {
        return Err(OrbitError::Execution(format!(
            "task worktree '{}' has no changes to commit",
            workspace_path.display()
        )));
    }

    let message = task_commit_message(&task.task_type, &task.title, &task.id, &summary);
    git_success(&workspace_path, &["commit", "-m", &message])?;
    let commit_sha = git_output(&workspace_path, &["rev-parse", "HEAD"])?;
    host.apply_task_automation_update(
        task_id,
        TaskAutomationUpdate {
            commit_message: Some(message.clone()),
            changed_files: Some(changed_files.clone()),
            ..TaskAutomationUpdate::default()
        },
    )?;

    Ok(json!({
        "repo_root": repo_root.to_string_lossy().to_string(),
        "workspace_path": workspace_path.to_string_lossy().to_string(),
        "branch": actual_branch.trim(),
        "commit_message": message,
        "commit_sha": commit_sha,
        "changed_files": changed_files,
    }))
}

fn merge_pr_from_task<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let task = host.get_task(task_id)?;
    let repo_root = canonicalize_existing_dir(
        &input_string_field(input, "repo_root")
            .or_else(|| input_workspace_path(input))
            .or_else(|| task.repo_root.clone())
            .or_else(|| task.workspace_path.clone())
            .ok_or_else(|| {
                OrbitError::InvalidInput(
                    "merge_pr_from_task requires input.repo_root, input.workspace_path, task.repo_root, or task.workspace_path"
                        .to_string(),
                )
            })?,
        "repo_root",
    )?;
    let pr_number = input_string_field(input, "pr_number")
        .or_else(|| task.pr_number.clone())
        .ok_or_else(|| {
            OrbitError::InvalidInput(
                "merge_pr_from_task requires input.pr_number or task.pr_number".to_string(),
            )
        })?;
    let head = input_string_field(input, "branch")
        .or_else(|| task.branch.clone())
        .ok_or_else(|| {
            OrbitError::Execution(format!(
                "task '{}' does not have a branch for PR merge",
                task.id
            ))
        })?;
    let base = input_string_field(input, "base").unwrap_or_else(|| "agent-main".to_string());
    let review_decision = resolve_review_decision(&repo_root, &pr_number)?;
    if review_decision != "APPROVED" {
        return Err(OrbitError::Execution(format!(
            "pull request '{pr_number}' is not approved (review_decision={review_decision})"
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
    let strategy = input_string_field(input, "strategy").unwrap_or_else(|| "squash".to_string());
    host.run_tool_with_context_and_role(
        "github.pr.merge",
        json!({
            "pr": pr_number.clone(),
            "strategy": strategy,
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
            pr_number: Some(pr_number.clone()),
            ..TaskAutomationUpdate::default()
        },
    )?;

    Ok(json!({
        "pr_number": pr_number,
        "merged": true,
        "review_decision": review_decision,
    }))
}

fn open_pr_from_task<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let task = host.get_task(task_id)?;
    let repo_root = canonicalize_existing_dir(
        &input_string_field(input, "repo_root")
            .or_else(|| input_workspace_path(input))
            .or_else(|| task.repo_root.clone())
            .ok_or_else(|| {
                OrbitError::InvalidInput(
                    "open_pr_from_task requires input.repo_root, input.workspace_path, or task.repo_root"
                        .to_string(),
                )
    })?,
    "repo_root",
)?;
    let branch = input_string_field(input, "branch");
    let base = input_string_field(input, "base").unwrap_or_else(|| "agent-main".to_string());
    let commit_message = input_string_field(input, "commit_message")
        .or_else(|| task.commit_message.clone())
        .unwrap_or_default();
    let changed_files = match input.get("changed_files") {
        Some(_) => input_string_array_field(input, "changed_files")?,
        None => task.changed_files.clone().unwrap_or_default(),
    };
    let head = branch.or_else(|| task.branch.clone()).ok_or_else(|| {
        OrbitError::Execution(format!(
            "task '{}' does not have a branch for PR creation",
            task.id
        ))
    })?;
    let freshness = ensure_branch_fresh_against_base(&repo_root, &head, &base)?;
    let body = format!(
        "## Changes\n{}\n\n## Branch Freshness\n- Base ref: `{}`\n- Head ref: `{}`\n- Behind base: {}\n- Ahead of base: {}\n\n## Files Changed\n{}",
        commit_message,
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
            branch: Some(head.clone()),
            pr_number: Some(pr_number.clone()),
            execution_summary: Some(body.clone()),
            ..TaskAutomationUpdate::default()
        },
    )?;

    Ok(json!({
        "pr_url": pr_create["url"].clone(),
        "pr_number": pr_number,
        "title": title,
        "body": body,
        "base": base,
        "head": head,
    }))
}

fn finalize_task_worktree(input: &Value) -> Result<Value, OrbitError> {
    let workspace_path = canonicalize_existing_dir(
        &input_workspace_path(input).ok_or_else(|| {
            OrbitError::InvalidInput(
                "finalize_task_worktree requires input.workspace_path".to_string(),
            )
        })?,
        "workspace_path",
    )?;
    let repo_root = canonicalize_existing_dir(&input_repo_root(input)?, "repo_root")?;
    let cleanup_strategy = if workspace_path == repo_root {
        "main_checkout_unchanged"
    } else {
        "retained"
    };
    Ok(json!({
        "workspace_path": workspace_path.to_string_lossy().to_string(),
        "repo_root": repo_root.to_string_lossy().to_string(),
        "cleanup_strategy": cleanup_strategy,
    }))
}

fn required_input_string<'a>(input: &'a Value, key: &str) -> Result<&'a str, OrbitError> {
    input
        .as_object()
        .and_then(|map| map.get(key))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| OrbitError::InvalidInput(format!("missing required input.{key}")))
}

fn input_string_field(input: &Value, key: &str) -> Option<String> {
    input
        .as_object()
        .and_then(|map| map.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn input_string_array_field(input: &Value, key: &str) -> Result<Vec<String>, OrbitError> {
    let Some(values) = input
        .as_object()
        .and_then(|map| map.get(key))
        .and_then(Value::as_array)
    else {
        return Ok(Vec::new());
    };

    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .ok_or_else(|| {
                    OrbitError::InvalidInput(format!(
                        "input.{key}[{index}] must be a non-empty string"
                    ))
                })
        })
        .collect()
}

fn input_workspace_path(input: &Value) -> Option<String> {
    input
        .as_object()
        .and_then(|map| map.get("workspace_path"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn input_repo_root(input: &Value) -> Result<String, OrbitError> {
    input_string_field(input, "repo_root")
        .or_else(|| input_workspace_path(input))
        .ok_or_else(|| OrbitError::InvalidInput("missing required input.repo_root".to_string()))
}

fn resolve_review_decision(repo_root: &Path, pr_number: &str) -> Result<String, OrbitError> {
    fetch_review_decision_from_gh(repo_root, pr_number)
}

fn normalize_review_decision(value: &str) -> String {
    match value.trim().to_ascii_uppercase().as_str() {
        "APPROVED" | "APPROVE" => "APPROVED".to_string(),
        "REQUEST-CHANGES" | "REQUEST_CHANGES" | "CHANGES_REQUESTED" => {
            "CHANGES_REQUESTED".to_string()
        }
        "COMMENT" | "COMMENTED" => "COMMENTED".to_string(),
        other => other.to_string(),
    }
}

fn fetch_review_decision_from_gh(repo_root: &Path, pr_number: &str) -> Result<String, OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "gh".to_string(),
            args: vec![
                "pr".to_string(),
                "view".to_string(),
                pr_number.to_string(),
                "--json".to_string(),
                "reviewDecision".to_string(),
            ],
            current_dir: Some(repo_root.to_string_lossy().to_string()),
            timeout_ms: Some(15_000),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
        debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        return Err(OrbitError::Execution(format!(
            "gh pr view failed while fetching reviewDecision for '{pr_number}': {}",
            result.stderr.trim()
        )));
    }

    let payload: Value = serde_json::from_str(&result.stdout).map_err(|error| {
        OrbitError::Execution(format!(
            "failed to parse gh pr view reviewDecision output for '{pr_number}': {error}"
        ))
    })?;
    match payload.get("reviewDecision") {
        // GitHub returns null when no reviews exist or branch protection doesn't require them.
        Some(Value::Null) | None => Ok("NONE".to_string()),
        Some(v) => v
            .as_str()
            .map(normalize_review_decision)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                OrbitError::Execution(format!(
                    "gh pr view returned unexpected reviewDecision type for '{pr_number}'"
                ))
            }),
    }
}

fn canonicalize_existing_dir(raw: &str, field_name: &str) -> Result<PathBuf, OrbitError> {
    let path = PathBuf::from(raw);
    if !path.exists() {
        return Err(OrbitError::InvalidInput(format!(
            "{field_name} does not exist: {raw}"
        )));
    }
    if !path.is_dir() {
        return Err(OrbitError::InvalidInput(format!(
            "{field_name} is not a directory: {raw}"
        )));
    }
    path.canonicalize().map_err(|error| {
        OrbitError::InvalidInput(format!(
            "failed to canonicalize {field_name} '{raw}': {error}"
        ))
    })
}

fn resolve_task_worktree_path(repo_root: &Path, task_id: &str) -> Result<PathBuf, OrbitError> {
    let repo_name = repo_root
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            OrbitError::Execution(format!(
                "cannot derive repository name from '{}'",
                repo_root.display()
            ))
        })?;
    let base_root = match std::env::var("ORBIT_WORKTREE_ROOT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        Some(value) => PathBuf::from(value),
        None => {
            let parent = repo_root.parent().ok_or_else(|| {
                OrbitError::Execution(format!(
                    "cannot derive worktree root from '{}'",
                    repo_root.display()
                ))
            })?;
            parent.parent().unwrap_or(parent).join("worktrees")
        }
    };
    Ok(base_root.join(repo_name).join(task_id))
}

fn ensure_existing_task_worktree(
    worktree_path: &Path,
    expected_branch: &str,
) -> Result<(), OrbitError> {
    let inside = git_output(worktree_path, &["rev-parse", "--is-inside-work-tree"])?;
    if inside.trim() != "true" {
        return Err(OrbitError::Execution(format!(
            "worktree path exists but is not a git worktree: {}",
            worktree_path.display()
        )));
    }
    let current_branch = git_output(worktree_path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if current_branch.trim() != expected_branch {
        return Err(OrbitError::Execution(format!(
            "existing worktree '{}' is on branch '{}' but '{}' was expected",
            worktree_path.display(),
            current_branch.trim(),
            expected_branch
        )));
    }
    Ok(())
}

fn fetch_remote_base(repo_root: &Path, base: &str) {
    let _ = run_process(
        &ExecRequest {
            program: "git".to_string(),
            args: vec!["fetch".to_string(), "origin".to_string(), base.to_string()],
            current_dir: Some(repo_root.to_string_lossy().to_string()),
            timeout_ms: Some(60_000),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
        debug: false,
        },
        &NoSandbox,
    );
}

fn ensure_branch_fresh_against_base(
    repo_root: &Path,
    head: &str,
    base: &str,
) -> Result<BranchFreshness, OrbitError> {
    fetch_remote_base(repo_root, base);
    let base_ref = resolve_worktree_start_point(repo_root, base)?;
    let divergence = git_output(
        repo_root,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{base_ref}...{head}"),
        ],
    )?;
    let mut parts = divergence.split_whitespace();
    let commits_behind = parse_divergence_count(parts.next(), "behind", base, head)?;
    let commits_ahead = parse_divergence_count(parts.next(), "ahead", base, head)?;
    if parts.next().is_some() {
        return Err(OrbitError::Execution(format!(
            "unexpected git divergence output while comparing '{head}' to '{base_ref}': {divergence}"
        )));
    }

    if commits_behind > 0 {
        return Err(OrbitError::Execution(format!(
            "task branch '{head}' is behind base '{base_ref}' by {commits_behind} commit(s); refresh the task branch before opening or merging the PR"
        )));
    }

    Ok(BranchFreshness {
        base_ref,
        head_ref: head.to_string(),
        commits_behind,
        commits_ahead,
    })
}

fn parse_divergence_count(
    value: Option<&str>,
    label: &str,
    base: &str,
    head: &str,
) -> Result<u64, OrbitError> {
    let raw = value.ok_or_else(|| {
        OrbitError::Execution(format!(
            "missing {label} divergence count while comparing '{head}' to '{base}'"
        ))
    })?;
    raw.parse::<u64>().map_err(|error| {
        OrbitError::Execution(format!(
            "invalid {label} divergence count '{raw}' while comparing '{head}' to '{base}': {error}"
        ))
    })
}

fn resolve_worktree_start_point(repo_root: &Path, base: &str) -> Result<String, OrbitError> {
    let remote_base = format!("origin/{base}");
    if git_command_success(
        repo_root,
        &[
            "rev-parse",
            "--verify",
            &format!("{remote_base}^{{commit}}"),
        ],
    )? {
        return Ok(remote_base);
    }

    if git_command_success(
        repo_root,
        &["rev-parse", "--verify", &format!("{base}^{{commit}}")],
    )? {
        return Ok(base.to_string());
    }

    Err(OrbitError::Execution(format!(
        "unable to resolve base ref '{base}' for task worktree creation"
    )))
}

fn create_or_attach_task_worktree(
    repo_root: &Path,
    worktree_path: &Path,
    branch: &str,
    start_point: &str,
) -> Result<(), OrbitError> {
    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            OrbitError::Execution(format!(
                "failed to create task worktree directory '{}': {error}",
                parent.display()
            ))
        })?;
    }

    if git_command_success(
        repo_root,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )? {
        git_success(
            repo_root,
            &["worktree", "add", &worktree_path.to_string_lossy(), branch],
        )
    } else {
        git_success(
            repo_root,
            &[
                "worktree",
                "add",
                "-b",
                branch,
                &worktree_path.to_string_lossy(),
                start_point,
            ],
        )
    }
}

fn ensure_no_unmerged_changes(workspace_path: &Path) -> Result<(), OrbitError> {
    let status = git_output(workspace_path, &["status", "--porcelain"])?;
    for line in status.lines() {
        if line.len() < 2 {
            continue;
        }
        let bytes = line.as_bytes();
        let x = bytes[0] as char;
        let y = bytes[1] as char;
        if x == 'U' || y == 'U' || (x == 'A' && y == 'A') || (x == 'D' && y == 'D') {
            return Err(OrbitError::Execution(format!(
                "task worktree '{}' has unresolved merge conflicts",
                workspace_path.display()
            )));
        }
    }
    Ok(())
}

fn git_output_paths(current_dir: &Path, args: &[&str]) -> Result<Vec<String>, OrbitError> {
    let raw = git_output_raw(current_dir, args)?;
    Ok(raw
        .split('\0')
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn git_output(current_dir: &Path, args: &[&str]) -> Result<String, OrbitError> {
    Ok(git_output_raw(current_dir, args)?.trim().to_string())
}

fn git_output_raw(current_dir: &Path, args: &[&str]) -> Result<String, OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "git".to_string(),
            args: args.iter().map(|value| (*value).to_string()).collect(),
            current_dir: Some(current_dir.to_string_lossy().to_string()),
            timeout_ms: Some(30_000),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
        debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        return Err(OrbitError::Execution(format!(
            "git {} failed in '{}': {}",
            args.join(" "),
            current_dir.display(),
            result.stderr.trim()
        )));
    }

    Ok(result.stdout)
}

fn git_success(current_dir: &Path, args: &[&str]) -> Result<(), OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "git".to_string(),
            args: args.iter().map(|value| (*value).to_string()).collect(),
            current_dir: Some(current_dir.to_string_lossy().to_string()),
            timeout_ms: Some(30_000),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
        debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        return Err(OrbitError::Execution(format!(
            "git {} failed in '{}': {}",
            args.join(" "),
            current_dir.display(),
            result.stderr.trim()
        )));
    }

    Ok(())
}

fn git_command_success(current_dir: &Path, args: &[&str]) -> Result<bool, OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "git".to_string(),
            args: args.iter().map(|value| (*value).to_string()).collect(),
            current_dir: Some(current_dir.to_string_lossy().to_string()),
            timeout_ms: Some(30_000),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
        debug: false,
        },
        &NoSandbox,
    )?;
    Ok(result.success)
}

fn task_commit_message(task_type: &TaskType, title: &str, task_id: &str, body: &str) -> String {
    let prefix = match task_type {
        TaskType::Task | TaskType::Feature => "feat",
        TaskType::Issue => "fix",
        TaskType::Chore => "chore",
        TaskType::Refactor => "refactor",
    };
    let summary = title.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{prefix}: {summary} [{task_id}]\n\n{body}")
}

fn json_number_to_string(value: &Value) -> Option<String> {
    value
        .as_i64()
        .map(|number| number.to_string())
        .or_else(|| value.as_u64().map(|number| number.to_string()))
        .or_else(|| value.as_str().map(ToOwned::to_owned))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use std::sync::{Mutex, OnceLock};

    use chrono::Utc;
    use orbit_tools::ToolContext;
    use orbit_types::{Activity, JobTargetType, OrbitEvent, Role, Task, TaskPriority};
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[derive(Debug, Clone)]
    struct ToolInvocation {
        name: String,
        input: Value,
        role: Role,
        tool_context: ToolContext,
    }

    #[derive(Default)]
    struct FakeHost {
        task: RefCell<Option<Task>>,
        tool_invocations: RefCell<Vec<ToolInvocation>>,
        automation_updates: RefCell<Vec<TaskAutomationUpdate>>,
    }

    impl FakeHost {
        fn new(task: Task) -> Self {
            Self {
                task: RefCell::new(Some(task)),
                tool_invocations: RefCell::new(Vec::new()),
                automation_updates: RefCell::new(Vec::new()),
            }
        }
    }

    impl TaskHost for FakeHost {
        fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
            let task = self
                .task
                .borrow()
                .clone()
                .ok_or_else(|| OrbitError::TaskNotFound(task_id.to_string()))?;
            if task.id != task_id {
                return Err(OrbitError::TaskNotFound(task_id.to_string()));
            }
            Ok(task)
        }

        fn start_task(
            &self,
            _task_id: &str,
            _note: Option<String>,
            _comment: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!("start_task is not used in automation merge tests")
        }

        fn update_task_from_activity(
            &self,
            _task_id: &str,
            _status: TaskStatus,
            _execution_summary: Option<String>,
            _files_changed: Vec<String>,
            _comment: Option<String>,
            _note: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!("update_task_from_activity is not used in automation merge tests")
        }

        fn apply_task_automation_update(
            &self,
            _task_id: &str,
            update: TaskAutomationUpdate,
        ) -> Result<(), OrbitError> {
            self.automation_updates.borrow_mut().push(update);
            Ok(())
        }
    }

    impl RuntimeHost for FakeHost {
        fn record_event(&self, _event: OrbitEvent) -> Result<(), OrbitError> {
            Ok(())
        }

        fn repo_root(&self) -> Result<String, OrbitError> {
            Err(OrbitError::Execution(
                "repo_root is not used in automation merge tests".to_string(),
            ))
        }

        fn validate_activity_target_exists(
            &self,
            _target_type: JobTargetType,
            _target_id: &str,
        ) -> Result<Activity, OrbitError> {
            unimplemented!(
                "validate_activity_target_exists is not used in automation merge tests"
            )
        }

        fn run_tool_with_context_and_role(
            &self,
            name: &str,
            input: Value,
            role: Role,
            tool_context: ToolContext,
        ) -> Result<Value, OrbitError> {
            self.tool_invocations.borrow_mut().push(ToolInvocation {
                name: name.to_string(),
                input,
                role,
                tool_context,
            });
            Ok(json!({}))
        }
    }

    struct PathGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        original_path: Option<String>,
    }

    impl Drop for PathGuard {
        fn drop(&mut self) {
            match self.original_path.take() {
                Some(path) => unsafe { std::env::set_var("PATH", path) },
                None => unsafe { std::env::remove_var("PATH") },
            }
        }
    }

    fn path_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn prepend_path(dir: &Path) -> String {
        let mut entries = vec![dir.to_string_lossy().to_string()];
        if let Some(existing) = std::env::var_os("PATH") {
            entries.push(existing.to_string_lossy().to_string());
        }
        entries.join(":")
    }

    fn install_fake_gh(bin_dir: &Path, decisions: &[(&str, &str)]) {
        let decision_map = decisions
            .iter()
            .map(|(pr_number, decision)| {
                // Pass "null" (the literal string) to emit a JSON null value unquoted.
                let payload = if *decision == "null" {
                    "{\"reviewDecision\":null}".to_string()
                } else {
                    format!("{{\"reviewDecision\":\"{decision}\"}}")
                };
                (pr_number.to_string(), payload)
            })
            .collect::<HashMap<_, _>>();
        let cases = decision_map
            .iter()
            .map(|(pr_number, payload)| {
                format!("  {pr_number}) printf '%s' '{payload}'; exit 0 ;;\n")
            })
            .collect::<String>();
        let script = format!(
            concat!(
                "#!/bin/sh\n",
                "if [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ] && [ \"$4\" = \"--json\" ] && [ \"$5\" = \"reviewDecision\" ]; then\n",
                "  case \"$3\" in\n",
                "{cases}",
                "  esac\n",
                "fi\n",
                "printf '%s\\n' \"unexpected gh args: $*\" >&2\n",
                "exit 1\n"
            ),
            cases = cases
        );
        let gh_path = bin_dir.join("gh");
        fs::write(&gh_path, script).expect("write fake gh");
        #[cfg(unix)]
        fs::set_permissions(&gh_path, fs::Permissions::from_mode(0o755)).expect("chmod gh");
    }

    fn use_fake_gh(decisions: &[(&str, &str)]) -> (TempDir, PathGuard) {
        let bin_dir = tempfile::tempdir().expect("temp gh dir");
        install_fake_gh(bin_dir.path(), decisions);
        let lock = path_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let original_path = std::env::var("PATH").ok();
        unsafe { std::env::set_var("PATH", prepend_path(bin_dir.path())) };
        (
            bin_dir,
            PathGuard {
                _lock: lock,
                original_path,
            },
        )
    }

    fn git(repo_root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .status()
            .expect("run git");
        assert!(status.success(), "git {:?} failed", args);
    }

    fn init_repo() -> TempDir {
        let repo_dir = tempfile::tempdir().expect("temp repo dir");
        git(repo_dir.path(), &["init", "--initial-branch=agent-main"]);
        git(repo_dir.path(), &["config", "user.name", "Orbit Tests"]);
        git(repo_dir.path(), &["config", "user.email", "orbit-tests@example.com"]);
        fs::write(repo_dir.path().join("README.md"), "orbit\n").expect("write readme");
        git(repo_dir.path(), &["add", "README.md"]);
        git(repo_dir.path(), &["commit", "-m", "init"]);
        git(repo_dir.path(), &["checkout", "-b", "orbit/T20260320-021158"]);
        git(repo_dir.path(), &["checkout", "agent-main"]);
        repo_dir
    }

    fn test_task(repo_root: &Path) -> Task {
        Task {
            id: "T20260320-021158".to_string(),
            title: "merge_pr_from_task uses GitHub review decision".to_string(),
            description: "desc".to_string(),
            plan: "plan".to_string(),
            execution_summary: String::new(),
            context_files: vec!["orbit-engine/src/executor/automation.rs".to_string()],
            workspace_path: Some(repo_root.to_string_lossy().to_string()),
            repo_root: Some(repo_root.to_string_lossy().to_string()),
            assigned_to: None,
            created_by: Some("test".to_string()),
            status: TaskStatus::Review,
            priority: TaskPriority::High,
            task_type: TaskType::Issue,
            branch: Some("orbit/T20260320-021158".to_string()),
            commit_message: None,
            changed_files: None,
            pr_number: Some("18".to_string()),
            proposed_by: None,
            complexity: None,
            comments: vec![],
            history: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn merge_pr_from_task_prefers_github_approval_over_agent_reported_commented() {
        let repo_dir = init_repo();
        let (_gh_dir, _path_guard) = use_fake_gh(&[("18", "APPROVED")]);
        let host = FakeHost::new(test_task(repo_dir.path()));
        let canonical_repo_root = repo_dir.path().canonicalize().expect("canonical repo root");

        let result = merge_pr_from_task(
            &host,
            &json!({
                "task_id": "T20260320-021158",
                "repo_root": repo_dir.path().to_string_lossy().to_string(),
                "review_decision": "COMMENTED",
                "action": "COMMENTED",
                "pr_number": "18",
            }),
        )
        .expect("merge should succeed");

        assert_eq!(result["merged"], json!(true));
        assert_eq!(result["review_decision"], json!("APPROVED"));

        let tool_invocations = host.tool_invocations.borrow();
        assert_eq!(tool_invocations.len(), 1);
        assert_eq!(tool_invocations[0].name, "github.pr.merge");
        assert_eq!(tool_invocations[0].input["pr"], json!("18"));
        assert_eq!(tool_invocations[0].input["strategy"], json!("squash"));
        assert_eq!(tool_invocations[0].role, Role::Admin);
        assert_eq!(
            tool_invocations[0].tool_context.cwd.as_deref(),
            Some(canonical_repo_root.to_string_lossy().as_ref())
        );

        let automation_updates = host.automation_updates.borrow();
        assert_eq!(automation_updates.len(), 1);
        assert_eq!(automation_updates[0].status, Some(TaskStatus::Done));
        assert_eq!(automation_updates[0].pr_number.as_deref(), Some("18"));
    }

    #[test]
    fn merge_pr_from_task_blocks_when_github_reports_commented_even_if_agent_reported_approved() {
        let repo_dir = init_repo();
        let (_gh_dir, _path_guard) = use_fake_gh(&[("18", "COMMENTED")]);
        let host = FakeHost::new(test_task(repo_dir.path()));

        let error = merge_pr_from_task(
            &host,
            &json!({
                "task_id": "T20260320-021158",
                "repo_root": repo_dir.path().to_string_lossy().to_string(),
                "review_decision": "APPROVED",
                "action": "APPROVE",
                "pr_number": "18",
            }),
        )
        .expect_err("merge should fail");

        assert_eq!(
            error.to_string(),
            "execution failed: pull request '18' is not approved (review_decision=COMMENTED)"
        );
        assert!(host.tool_invocations.borrow().is_empty());
        assert!(host.automation_updates.borrow().is_empty());
    }

    #[test]
    fn merge_pr_from_task_returns_none_when_github_review_decision_is_null() {
        let repo_dir = init_repo();
        let (_gh_dir, _path_guard) = use_fake_gh(&[("20", "null")]);
        let mut task = test_task(repo_dir.path());
        task.id = "T20260320-025301".to_string();
        task.pr_number = Some("20".to_string());
        let host = FakeHost::new(task);

        let error = merge_pr_from_task(
            &host,
            &json!({
                "task_id": "T20260320-025301",
                "repo_root": repo_dir.path().to_string_lossy().to_string(),
                "pr_number": "20",
            }),
        )
        .expect_err("merge should fail when reviewDecision is null");

        assert_eq!(
            error.to_string(),
            "execution failed: pull request '20' is not approved (review_decision=NONE)"
        );
        assert!(host.tool_invocations.borrow().is_empty());
        assert!(host.automation_updates.borrow().is_empty());
    }
}
