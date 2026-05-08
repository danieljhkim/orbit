use std::path::Path;

use orbit_common::types::{
    ExternalRef, OrbitError, ReviewThreadStatus, Role, Task, TaskStatus,
    normalize_optional_attribution_label,
};
use orbit_store::pr_scoreboard;
use orbit_tools::ToolContext;
use serde_json::{Value, json};

use crate::context::{PrConfig, RuntimeHost, TaskAutomationUpdate, TaskHost};

use super::freshness::{ensure_branch_fresh_against_base, ensure_branch_rebased_onto_base};
use super::git::{base_sync_mode_from_input, git_output};
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
    let batch_id = required_batch_id(input, "git_merge")?;
    if host
        .list_tasks_filtered(None, None, None, Some(batch_id), None, None)?
        .is_empty()
    {
        return Ok(json!({}));
    }

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

    let batch_tasks = host.list_tasks_filtered(None, None, None, Some(batch_id), None, None)?;
    if batch_tasks.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "merge_batch_pr: no tasks found for batch_id '{batch_id}'"
        )));
    }

    // Find the GitHub PR external ref from the first task that has one.
    let pr_number = batch_tasks
        .iter()
        .find_map(Task::github_pr_number)
        .ok_or_else(|| {
            OrbitError::InvalidInput(
                "merge_batch_pr: no task in batch has a github-pr external ref".to_string(),
            )
        })?
        .to_string();

    let workspace_path = resolve_batch_workspace_path(host, input, batch_id)?;

    // Get the current branch from the workspace
    let head = git_output(&workspace_path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let head = head.trim().to_string();
    let base = input_string_field(input, "base").unwrap_or_else(|| "main".to_string());
    let base_sync_mode = base_sync_mode_from_input(input)?;

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

    ensure_branch_fresh_against_base(&workspace_path, &head, &base, base_sync_mode)?;

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
        normalize_optional_attribution_label(
            task.implemented_by
                .as_deref()
                .or(task.model.as_deref())
                .or(task.created_by.as_deref()),
            task.model.as_deref(),
        )
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
                external_refs: vec![ExternalRef::github_pr(pr_number.clone())?],
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
            .list_tasks_filtered(None, None, None, Some(batch_id), None, None)?
            .into_iter()
            .map(|task| task.id)
            .collect(),
    };

    if completed_task_ids.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "open_batch_pr: no tasks found for batch_id '{batch_id}'"
        )));
    }

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
    ensure_completed_tasks_have_meaningful_execution_summaries(&completed_tasks)?;

    let head = git_output(&workspace_path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let head = head.trim().to_string();
    let base = input_string_field(input, "base").unwrap_or_else(|| "main".to_string());
    let base_sync_mode = base_sync_mode_from_input(input)?;

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

    let title = input_string_field(input, "title")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default_pr_title(&completed_tasks));
    let pr_config = host.pr_config();
    let body = input_string_field(input, "body")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            build_batch_pr_body(&completed_tasks, &freshness, &changed_files, &pr_config)
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
                external_refs: vec![ExternalRef::github_pr(pr_number.clone())?],
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

/// Builds the generated PR body. One-task PRs use the task-contract-first
/// layout; multi-task callers intentionally keep the historical batch layout
/// until those legacy paths are retired.
fn build_batch_pr_body(
    tasks: &[Task],
    freshness: &super::freshness::BranchFreshness,
    changed_files: &[&str],
    pr_config: &PrConfig,
) -> String {
    let mut body = if let [task] = tasks {
        build_single_task_pr_body(task, freshness, pr_config)
    } else {
        build_legacy_batch_pr_body(tasks, freshness, changed_files, pr_config)
    };

    if let Some(signature) = batch_pr_signature(tasks) {
        body.push_str("\n\n");
        body.push_str(&signature);
    }

    body
}

fn build_single_task_pr_body(
    task: &Task,
    freshness: &super::freshness::BranchFreshness,
    pr_config: &PrConfig,
) -> String {
    let mut sections = vec![render_single_task_section(task, pr_config)];

    if let Some(execution_summary) = meaningful_execution_summary(&task.execution_summary) {
        sections.push(render_execution_summary_section(execution_summary));
    }

    sections.push(render_validation_section());
    sections.push(render_branch_freshness_section(freshness));
    sections.join("\n\n")
}

fn build_legacy_batch_pr_body(
    tasks: &[Task],
    freshness: &super::freshness::BranchFreshness,
    changed_files: &[&str],
    pr_config: &PrConfig,
) -> String {
    let task_sections = tasks
        .iter()
        .map(|task| render_legacy_task_section(task, pr_config))
        .collect::<Vec<_>>()
        .join("\n");
    let changed_files_section = changed_files
        .iter()
        .map(|file| format!("- `{file}`"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "## Tasks\n{}\n\n## Branch Freshness\n- Base ref: `{}`\n- Head ref: `{}`\n- Behind base: {}\n- Ahead of base: {}\n\n## Files Changed\n{}",
        task_sections,
        freshness.base_ref,
        freshness.head_ref,
        freshness.commits_behind,
        freshness.commits_ahead,
        changed_files_section
    )
}

fn render_single_task_section(task: &Task, pr_config: &PrConfig) -> String {
    let acceptance_criteria = render_acceptance_criteria(&task.acceptance_criteria);

    format!(
        "## Task\n\n{}\n\n### Description\n\n{}\n\n### Acceptance Criteria\n\n{}",
        render_single_task_line(task, pr_config),
        task.description,
        acceptance_criteria
    )
}

fn render_execution_summary_section(execution_summary: &str) -> String {
    format!(
        "## Execution Summary\n\n<details>\n<summary>Click to expand</summary>\n\n{execution_summary}\n\n</details>"
    )
}

fn render_validation_section() -> String {
    "## Validation\n\n- Not reported".to_string()
}

fn render_branch_freshness_section(freshness: &super::freshness::BranchFreshness) -> String {
    format!(
        "## Branch Freshness\n\n- Base ref: `{}`\n- Head ref: `{}`\n- Behind base: {}\n- Ahead of base: {}",
        freshness.base_ref, freshness.head_ref, freshness.commits_behind, freshness.commits_ahead
    )
}

fn render_acceptance_criteria(criteria: &[String]) -> String {
    criteria
        .iter()
        .map(|criterion| format!("- {criterion}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_legacy_task_section(task: &Task, pr_config: &PrConfig) -> String {
    let line = render_task_line(task, pr_config);
    match meaningful_execution_summary(&task.execution_summary) {
        Some(execution_summary) => {
            format!(
                "{line}\n  <details><summary>Execution Summary</summary>\n\n{execution_summary}\n\n  </details>"
            )
        }
        None => line,
    }
}

fn meaningful_execution_summary(summary: &str) -> Option<&str> {
    let trimmed = summary.trim();
    if trimmed.is_empty() || is_placeholder_execution_summary(trimmed) {
        None
    } else {
        Some(trimmed)
    }
}

fn is_placeholder_execution_summary(summary: &str) -> bool {
    let normalized = summary
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let lower = normalized.to_ascii_lowercase();
    let stripped = lower.trim_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace());
    stripped.is_empty()
        || matches!(
            stripped,
            "todo"
                | "tbd"
                | "n/a"
                | "na"
                | "none"
                | "placeholder"
                | "execution summary"
                | "summary"
                | "no execution summary"
                | "no summary provided"
                | "no execution summary provided"
                | "to be authored by executing agent at start time"
        )
}

fn render_task_line(task: &Task, pr_config: &PrConfig) -> String {
    let title = task.title.trim();
    let task_ref = render_task_ref(task, pr_config);
    if title.is_empty() {
        format!("- {task_ref}")
    } else {
        format!("- {task_ref} {title}")
    }
}

fn render_single_task_line(task: &Task, pr_config: &PrConfig) -> String {
    let title = task.title.trim();
    let task_ref = render_task_ref(task, pr_config);
    if title.is_empty() {
        task_ref
    } else {
        format!("{task_ref} — {title}")
    }
}

fn render_task_ref(task: &Task, pr_config: &PrConfig) -> String {
    match task_url(task, pr_config) {
        Some(url) => format!("[{}]({url})", task.id),
        None => task.id.clone(),
    }
}

fn task_url(task: &Task, pr_config: &PrConfig) -> Option<String> {
    task.external_refs
        .iter()
        .find_map(|external_ref| external_ref.url.as_deref())
        .map(ToOwned::to_owned)
        .or_else(|| {
            pr_config
                .task_url_template
                .as_ref()
                .map(|template| template.replace("{task_id}", &task.id))
        })
}

fn default_pr_title(tasks: &[Task]) -> String {
    let first_task = tasks.first();
    let first_title = first_task
        .map(|task| task.title.trim())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| first_task.map(|task| task.id.as_str()).unwrap_or("Bundle"));
    if tasks.len() == 1 {
        first_title.to_string()
    } else {
        format!("[Bundle] {first_title}")
    }
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Mutex;

    use chrono::Utc;
    use orbit_common::types::{
        Activity, Job, JobTargetType, OrbitEvent, Role, TaskArtifact, TaskPriority, TaskType,
        push_external_ref_if_missing,
    };
    use orbit_tools::ToolContext;
    use serde_json::{Value, json};
    use tempfile::{TempDir, tempdir};

    use crate::context::{JobRunResult, RuntimeHost, TaskReadHost, TaskWriteHost};
    use crate::executor::registry::ActivityExecutorRegistry;

    use super::super::freshness::BranchFreshness;
    use super::*;

    #[derive(Clone, Debug)]
    struct ToolCall {
        name: String,
        input: Value,
    }

    struct PrOpenTestHost {
        tasks: Mutex<Vec<Task>>,
        tool_calls: Mutex<Vec<ToolCall>>,
        repo_root: PathBuf,
        data_root: PathBuf,
        scoreboard_dir: PathBuf,
        registry: ActivityExecutorRegistry,
    }

    impl PrOpenTestHost {
        fn new(tasks: Vec<Task>, repo_root: PathBuf) -> Self {
            let data_root = repo_root.join(".orbit-test-data");
            let scoreboard_dir = data_root.join("scoreboard");
            Self {
                tasks: Mutex::new(tasks),
                tool_calls: Mutex::new(Vec::new()),
                repo_root,
                data_root,
                scoreboard_dir,
                registry: ActivityExecutorRegistry::default(),
            }
        }

        fn tool_calls(&self) -> Vec<ToolCall> {
            self.tool_calls.lock().expect("tool calls lock").clone()
        }

        fn pr_create_body(&self) -> String {
            self.tool_calls()
                .into_iter()
                .find(|call| call.name == "github.pr.create")
                .and_then(|call| {
                    call.input
                        .get("body")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .expect("github.pr.create body")
        }
    }

    impl TaskReadHost for PrOpenTestHost {
        fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
            self.tasks
                .lock()
                .expect("tasks lock")
                .iter()
                .find(|task| task.id == task_id)
                .cloned()
                .ok_or_else(|| OrbitError::TaskNotFound(task_id.to_string()))
        }

        fn get_task_artifacts(&self, _task_id: &str) -> Result<Vec<TaskArtifact>, OrbitError> {
            Ok(Vec::new())
        }

        fn list_tasks_filtered(
            &self,
            status: Option<TaskStatus>,
            priority: Option<TaskPriority>,
            parent_id: Option<&str>,
            batch_id: Option<&str>,
            external_ref: Option<&orbit_common::types::ExternalRef>,
            has_external_ref_system: Option<&str>,
        ) -> Result<Vec<Task>, OrbitError> {
            Ok(self
                .tasks
                .lock()
                .expect("tasks lock")
                .iter()
                .filter(|task| status.is_none_or(|status| task.status == status))
                .filter(|task| priority.is_none_or(|priority| task.priority == priority))
                .filter(|task| {
                    parent_id.is_none_or(|parent_id| task.parent_id.as_deref() == Some(parent_id))
                })
                .filter(|task| {
                    batch_id.is_none_or(|batch_id| task.batch_id.as_deref() == Some(batch_id))
                })
                .filter(|task| {
                    external_ref.is_none_or(|external_ref| {
                        task.external_refs.iter().any(|candidate| {
                            candidate.system == external_ref.system
                                && candidate.id == external_ref.id
                        })
                    })
                })
                .filter(|task| {
                    has_external_ref_system.is_none_or(|system| {
                        task.external_refs
                            .iter()
                            .any(|candidate| candidate.system == system)
                    })
                })
                .cloned()
                .collect())
        }
    }

    impl TaskWriteHost for PrOpenTestHost {
        fn start_task(
            &self,
            _task_id: &str,
            _note: Option<String>,
            _comment: Option<String>,
        ) -> Result<Task, OrbitError> {
            Err(OrbitError::Execution(
                "start_task is not needed by pr_open tests".to_string(),
            ))
        }

        fn admit_task_for_workflow(
            &self,
            _task_id: &str,
            _workflow: &str,
        ) -> Result<Task, OrbitError> {
            Err(OrbitError::Execution(
                "admit_task_for_workflow is not needed by pr_open tests".to_string(),
            ))
        }

        fn update_task_from_activity(
            &self,
            _task_id: &str,
            _status: TaskStatus,
            _execution_summary: Option<String>,
            _comment: Option<String>,
            _note: Option<String>,
        ) -> Result<Task, OrbitError> {
            Err(OrbitError::Execution(
                "update_task_from_activity is not needed by pr_open tests".to_string(),
            ))
        }

        fn apply_task_automation_update(
            &self,
            task_id: &str,
            update: TaskAutomationUpdate,
        ) -> Result<(), OrbitError> {
            let mut tasks = self.tasks.lock().expect("tasks lock");
            let task = tasks
                .iter_mut()
                .find(|task| task.id == task_id)
                .ok_or_else(|| OrbitError::TaskNotFound(task_id.to_string()))?;
            if let Some(status) = update.status {
                task.status = status;
            }
            for external_ref in update.external_refs {
                push_external_ref_if_missing(&mut task.external_refs, external_ref);
            }
            if let Some(execution_summary) = update.execution_summary {
                task.execution_summary = execution_summary;
            }
            Ok(())
        }
    }

    impl RuntimeHost for PrOpenTestHost {
        fn record_event(&self, _event: OrbitEvent) -> Result<(), OrbitError> {
            Ok(())
        }

        fn repo_root(&self) -> Result<String, OrbitError> {
            Ok(self.repo_root.to_string_lossy().to_string())
        }

        fn data_root(&self) -> &Path {
            &self.data_root
        }

        fn activity_executor_registry(&self) -> &ActivityExecutorRegistry {
            &self.registry
        }

        fn run_job_now_with_input_debug(
            &self,
            _job_id: &str,
            _input: Value,
            _debug: bool,
        ) -> Result<JobRunResult, OrbitError> {
            Err(OrbitError::Execution(
                "run_job_now_with_input_debug is not needed by pr_open tests".to_string(),
            ))
        }

        fn validate_activity_target_exists(
            &self,
            _target_type: JobTargetType,
            _target_id: &str,
        ) -> Result<Activity, OrbitError> {
            Err(OrbitError::Execution(
                "validate_activity_target_exists is not needed by pr_open tests".to_string(),
            ))
        }

        fn get_job(&self, _job_id: &str) -> Result<Option<Job>, OrbitError> {
            Ok(None)
        }

        fn run_tool_with_context_and_role(
            &self,
            name: &str,
            input: Value,
            _role: Role,
            _tool_context: ToolContext,
        ) -> Result<Value, OrbitError> {
            self.tool_calls
                .lock()
                .expect("tool calls lock")
                .push(ToolCall {
                    name: name.to_string(),
                    input: input.clone(),
                });

            match name {
                "git.push" => Ok(json!({})),
                "github.pr.create" => Ok(json!({
                    "url": "https://github.example/orbit/orbit/pull/42"
                })),
                "github.pr.view" => Ok(json!({
                    "pull_request": { "number": 42 }
                })),
                other => Err(OrbitError::ToolNotFound(other.to_string())),
            }
        }

        fn maybe_create_failure_task(
            &self,
            _job_id: &str,
            _run_id: &str,
            _error_code: &str,
            _error_message: &str,
            _agent: Option<&str>,
            _model: Option<&str>,
        ) -> Result<(), OrbitError> {
            Ok(())
        }

        fn scoring_enabled(&self) -> bool {
            false
        }

        fn graph_editing(&self) -> bool {
            false
        }

        fn scoreboard_dir(&self) -> &Path {
            &self.scoreboard_dir
        }
    }

    fn task(id: &str, title: &str, execution_summary: &str) -> Task {
        let now = Utc::now();
        Task {
            id: id.to_string(),
            parent_id: None,
            title: title.to_string(),
            description: String::new(),
            acceptance_criteria: Vec::new(),
            dependencies: Vec::new(),
            plan: String::new(),
            execution_summary: execution_summary.to_string(),
            context_files: Vec::new(),
            workspace_path: None,
            repo_root: None,
            created_by: Some("gpt-5.5".to_string()),
            planned_by: None,
            implemented_by: None,
            agent: None,
            model: None,
            status: TaskStatus::Review,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Task,
            pr_status: None,
            external_refs: Vec::new(),
            source_task_id: None,
            batch_id: None,
            comments: Vec::new(),
            history: Vec::new(),
            review_threads: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    fn batch_task(id: &str, title: &str, execution_summary: &str) -> Task {
        let mut task = task(id, title, execution_summary);
        task.status = TaskStatus::InProgress;
        task.batch_id = Some("batch-1".to_string());
        task
    }

    fn task_with_contract(
        id: &str,
        title: &str,
        execution_summary: &str,
        description: &str,
        acceptance_criteria: &[String],
        task_url: Option<&str>,
    ) -> Task {
        let mut task = task(id, title, execution_summary);
        task.description = description.to_string();
        task.acceptance_criteria = acceptance_criteria.to_vec();
        if let Some(task_url) = task_url {
            task.external_refs = vec![
                ExternalRef::try_new(
                    "orbit-task".to_string(),
                    id.to_string(),
                    Some(task_url.to_string()),
                )
                .expect("task url external ref"),
            ];
        }
        task
    }

    fn freshness() -> BranchFreshness {
        BranchFreshness {
            base_ref: "main".to_string(),
            head_ref: "feature/task".to_string(),
            commits_behind: 0,
            commits_ahead: 2,
        }
    }

    fn test_pr_config(task_url_template: Option<&str>) -> PrConfig {
        PrConfig {
            task_url_template: task_url_template.map(ToOwned::to_owned),
        }
    }

    struct PrWorkspace {
        _temp: TempDir,
        repo: PathBuf,
    }

    fn pr_workspace() -> PrWorkspace {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo dir");
        git(&repo, &["init"]);
        git(&repo, &["checkout", "-b", "agent-main"]);
        git(&repo, &["config", "user.name", "Orbit Test"]);
        git(&repo, &["config", "user.email", "orbit-test@example.com"]);
        fs::write(repo.join("README.md"), "base\n").expect("write readme");
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-m", "base"]);
        git(&repo, &["checkout", "-b", "orbit/test-batch"]);
        fs::create_dir_all(repo.join("src")).expect("create src dir");
        fs::write(repo.join("src/lib.rs"), "pub fn changed() {}\n").expect("write lib");
        git(&repo, &["add", "src/lib.rs"]);
        git(&repo, &["commit", "-m", "change"]);

        PrWorkspace { _temp: temp, repo }
    }

    fn pr_open_input(repo: &Path, completed_task_ids: Vec<&str>) -> Value {
        json!({
            "workspace_path": repo.to_string_lossy(),
            "batch_id": "batch-1",
            "completed_task_ids": completed_task_ids,
            "base": "agent-main",
            "base_sync": "local",
        })
    }

    fn git(current_dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(current_dir)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {} failed in {}:\nstdout: {}\nstderr: {}",
            args.join(" "),
            current_dir.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[test]
    fn task_url_rendering_covers_template_and_external_ref_priority() {
        let plain = task_with_contract("T123", "Plain task", "done", "", &[], None);
        assert_eq!(task_url(&plain, &test_pr_config(None)), None);
        assert_eq!(
            render_single_task_line(&plain, &test_pr_config(None)),
            "T123 — Plain task"
        );

        let external = task_with_contract(
            "T124",
            "External task",
            "done",
            "",
            &[],
            Some("https://tracker.example/T124"),
        );
        assert_eq!(
            task_url(&external, &test_pr_config(None)).as_deref(),
            Some("https://tracker.example/T124")
        );
        assert_eq!(
            render_single_task_line(&external, &test_pr_config(None)),
            "[T124](https://tracker.example/T124) — External task"
        );

        let templated = task_with_contract("T125", "Templated task", "done", "", &[], None);
        assert_eq!(
            task_url(
                &templated,
                &test_pr_config(Some("https://orbit-cli.com/tasks/{task_id}"))
            )
            .as_deref(),
            Some("https://orbit-cli.com/tasks/T125")
        );
        assert_eq!(
            render_single_task_line(
                &templated,
                &test_pr_config(Some("https://orbit-cli.com/tasks/{task_id}"))
            ),
            "[T125](https://orbit-cli.com/tasks/T125) — Templated task"
        );

        let external_wins = task_with_contract(
            "T126",
            "External wins",
            "done",
            "",
            &[],
            Some("https://tracker.example/T126"),
        );
        assert_eq!(
            task_url(
                &external_wins,
                &test_pr_config(Some("https://orbit-cli.com/tasks/{task_id}"))
            )
            .as_deref(),
            Some("https://tracker.example/T126")
        );
        assert_eq!(
            render_single_task_line(
                &external_wins,
                &test_pr_config(Some("https://orbit-cli.com/tasks/{task_id}"))
            ),
            "[T126](https://tracker.example/T126) — External wins"
        );
    }

    #[test]
    fn default_pr_config_renders_plain_task_id_without_markdown_link_punctuation() {
        let body = build_batch_pr_body(
            &[task_with_contract(
                "T20260508-11",
                "Fix task links",
                "done",
                "Default config must not create broken links.",
                &["Task id renders as plain text.".to_string()],
                None,
            )],
            &freshness(),
            &[],
            &test_pr_config(None),
        );
        let task_line = body
            .lines()
            .find(|line| line.starts_with("T20260508-11"))
            .expect("plain task line");

        assert_eq!(task_line, "T20260508-11 — Fix task links");
        assert!(!task_line.contains(']'));
        assert!(!task_line.contains('('));
    }

    #[test]
    fn multi_task_pr_body_preserves_legacy_execution_summary_layout() {
        let first_summary = "## Status\nsuccess\n\n## Summary of Changes\n- Routed automation updates through system.";
        let second_summary =
            "## Status\nsuccess\n\n## Summary of Changes\n- Added PR body summary coverage.";
        let body = build_batch_pr_body(
            &[
                task("T20260427-24", "System attribution fix", first_summary),
                task("T20260427-25", "Review handoff", second_summary),
            ],
            &freshness(),
            &["crates/orbit-core/src/runtime/engine/task_host.rs"],
            &test_pr_config(None),
        );

        assert!(body.contains("- T20260427-24 System attribution fix"));
        assert!(body.contains("- T20260427-25 Review handoff"));
        assert_eq!(
            body.matches("<details><summary>Execution Summary</summary>")
                .count(),
            2
        );
        assert!(body.contains(first_summary));
        assert!(body.contains(second_summary));
    }

    #[test]
    fn multi_task_pr_body_preserves_legacy_placeholder_summary_omission() {
        let body = build_batch_pr_body(
            &[
                task("T20260427-32", "Include execution summaries", ""),
                task("T20260427-33", "Whitespace summary", "   \n"),
                task("T20260427-34", "Placeholder summary", "TODO"),
                task("T20260427-35", "Ellipsis summary", "..."),
            ],
            &freshness(),
            &[],
            &test_pr_config(None),
        );

        assert!(body.contains("- T20260427-32 Include execution summaries"));
        assert!(body.contains("- T20260427-33 Whitespace summary"));
        assert!(body.contains("- T20260427-34 Placeholder summary"));
        assert!(body.contains("- T20260427-35 Ellipsis summary"));
        assert!(!body.contains("<details><summary>Execution Summary</summary>"));
    }

    #[test]
    fn single_task_pr_body_matches_snapshot() {
        let body = build_batch_pr_body(
            &[task_with_contract(
                "T20260508-3",
                "Revise PR body template",
                "Reviewer context stays inline.\n\nSummary remains collapsible.",
                "Reviewers can inspect the task contract without leaving the PR.",
                &[
                    "Description is rendered verbatim.".to_string(),
                    "Acceptance criteria render as plain bullets.".to_string(),
                ],
                Some("https://orbit.example/tasks/T20260508-3"),
            )],
            &freshness(),
            &["crates/orbit-engine/src/executor/automation/pr.rs"],
            &test_pr_config(None),
        );

        assert_eq!(
            body,
            "## Task\n\n[T20260508-3](https://orbit.example/tasks/T20260508-3) — Revise PR body template\n\n### Description\n\nReviewers can inspect the task contract without leaving the PR.\n\n### Acceptance Criteria\n\n- Description is rendered verbatim.\n- Acceptance criteria render as plain bullets.\n\n## Execution Summary\n\n<details>\n<summary>Click to expand</summary>\n\nReviewer context stays inline.\n\nSummary remains collapsible.\n\n</details>\n\n## Validation\n\n- Not reported\n\n## Branch Freshness\n\n- Base ref: `main`\n- Head ref: `feature/task`\n- Behind base: 0\n- Ahead of base: 2\n\n*authored by: gpt-5.5*"
        );
    }

    #[test]
    fn single_task_pr_body_uses_contract_first_layout() {
        let body = build_batch_pr_body(
            &[task_with_contract(
                "T20260427-32",
                "Include execution summaries",
                "done",
                "Keep the task description near the review context.",
                &["Criterion one".to_string(), "Criterion two".to_string()],
                Some("https://orbit.example/tasks/T20260427-32"),
            )],
            &freshness(),
            &["crates/orbit-engine/src/executor/automation/pr.rs"],
            &test_pr_config(None),
        );

        let headings = body
            .lines()
            .filter(|line| line.starts_with("## "))
            .collect::<Vec<_>>();
        assert_eq!(
            headings,
            vec![
                "## Task",
                "## Execution Summary",
                "## Validation",
                "## Branch Freshness",
            ]
        );
        assert!(!body.contains("## Tasks"));
        assert!(!body.contains("## Status"));
        assert!(!body.contains("## Summary of Changes"));
        assert!(!body.contains("## Overall Assessment"));
        assert!(!body.contains("## Files Changed"));
        assert!(body.contains(
            "[T20260427-32](https://orbit.example/tasks/T20260427-32) — Include execution summaries"
        ));
        assert!(
            body.contains("### Description\n\nKeep the task description near the review context.")
        );
        assert!(body.contains("### Acceptance Criteria\n\n- Criterion one\n- Criterion two"));
        assert!(!body.contains("- [ ] Criterion one"));
        assert!(!body.contains("- [x] Criterion one"));
        assert!(body.contains("<summary>Click to expand</summary>"));
        assert!(body.contains("*authored by: gpt-5.5*"));
    }

    #[test]
    fn single_task_pr_body_omits_placeholder_execution_summary_section() {
        let body = build_batch_pr_body(
            &[task_with_contract(
                "T20260427-34",
                "Placeholder summary",
                "TODO",
                "Keep placeholder summaries out of generated PR bodies.",
                &["No details block is rendered.".to_string()],
                None,
            )],
            &freshness(),
            &[],
            &test_pr_config(None),
        );

        assert!(body.contains("## Task"));
        assert!(!body.contains("## Execution Summary"));
        assert!(!body.contains("<details>"));
        assert!(body.contains("## Validation"));
        assert!(body.contains("## Branch Freshness"));
    }

    #[test]
    fn pr_open_rejects_missing_execution_summary_before_create() {
        let workspace = pr_workspace();
        let host = PrOpenTestHost::new(
            vec![
                batch_task(
                    "T20260430-31A",
                    "First completed task",
                    "## Status\nsuccess\n\n## Summary of Changes\n- First task is complete.",
                ),
                batch_task("T20260430-31B", "Second completed task", "   \n"),
            ],
            workspace.repo.clone(),
        );

        let error = pr_open(
            &host,
            &pr_open_input(&workspace.repo, vec!["T20260430-31A", "T20260430-31B"]),
        )
        .expect_err("missing execution summary should reject PR creation");
        let message = error.to_string();

        assert!(message.contains("T20260430-31B"));
        assert!(message.contains("requires a meaningful persisted execution_summary"));
        assert!(message.contains("before opening the PR"));
        assert!(
            host.tool_calls()
                .iter()
                .all(|call| call.name != "github.pr.create")
        );
    }

    #[test]
    fn pr_open_generates_body_with_all_completed_task_summaries() {
        let workspace = pr_workspace();
        let first_summary =
            "## Status\nsuccess\n\n## Summary of Changes\n- Implemented the first bundle task.";
        let second_summary =
            "## Status\nsuccess\n\n## Summary of Changes\n- Implemented the second bundle task.";
        let host = PrOpenTestHost::new(
            vec![
                batch_task("T20260430-31A", "First completed task", first_summary),
                batch_task("T20260430-31B", "Second completed task", second_summary),
            ],
            workspace.repo.clone(),
        );

        pr_open(
            &host,
            &pr_open_input(&workspace.repo, vec!["T20260430-31A", "T20260430-31B"]),
        )
        .expect("pr_open should create PR");
        let body = host.pr_create_body();

        assert!(body.contains("- T20260430-31A First completed task"));
        assert!(body.contains(first_summary));
        assert!(body.contains("- T20260430-31B Second completed task"));
        assert!(body.contains(second_summary));
        assert_eq!(
            body.matches("<details><summary>Execution Summary</summary>")
                .count(),
            2
        );
    }

    #[test]
    fn pr_open_preserves_non_empty_explicit_body() {
        let workspace = pr_workspace();
        let host = PrOpenTestHost::new(
            vec![batch_task(
                "T20260430-31A",
                "First completed task",
                "## Status\nsuccess\n\n## Summary of Changes\n- Implemented the task.",
            )],
            workspace.repo.clone(),
        );
        let mut input = pr_open_input(&workspace.repo, vec!["T20260430-31A"]);
        input["body"] = json!("Custom reviewer handoff.");

        pr_open(&host, &input).expect("pr_open should create PR with explicit body");

        assert_eq!(host.pr_create_body(), "Custom reviewer handoff.");
    }
}
