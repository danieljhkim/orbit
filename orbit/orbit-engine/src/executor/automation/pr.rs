use orbit_store::pr_scoreboard;
use orbit_tools::ToolContext;
use orbit_types::{OrbitError, ReviewThreadStatus, Role, Task, TaskStatus};
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

use super::freshness::ensure_branch_fresh_against_base;
use super::git::git_output;
use super::input::{
    canonicalize_existing_dir, input_string_field, json_number_to_string, required_input_string,
};

pub(super) fn bootstrap_batch_review<H: TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let run_id = super::parallel::require_run_id(input, "bootstrap_batch_review")?;
    let existing_batch_tasks = host.list_tasks_filtered(None, None, None, Some(run_id))?;
    if !existing_batch_tasks.is_empty() {
        return Ok(json!({
            "batch_id": run_id,
            "task_count": existing_batch_tasks.len(),
            "tagged_count": 0,
        }));
    }

    let pr_number = required_input_string(input, "pr_number")?;
    let pr_tasks: Vec<Task> = host
        .list_tasks_filtered(None, None, None, None)?
        .into_iter()
        .filter(|task| task.pr_number.as_deref() == Some(pr_number))
        .collect();

    if pr_tasks.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "bootstrap_batch_review: no tasks found for pr_number '{pr_number}'"
        )));
    }

    for task in &pr_tasks {
        host.apply_task_automation_update(
            &task.id,
            TaskAutomationUpdate {
                batch_id: Some(run_id.to_string()),
                ..TaskAutomationUpdate::default()
            },
        )?;
    }

    Ok(json!({
        "batch_id": run_id,
        "task_count": pr_tasks.len(),
        "tagged_count": pr_tasks.len(),
    }))
}

pub(super) fn merge_batch_pr<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let batch_id = input
        .get("run_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            OrbitError::InvalidInput("merge_batch_pr requires input.run_id".to_string())
        })?;

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

    // Find repo_root/workspace_path from the first task that has one
    let repo_root = batch_tasks
        .iter()
        .find_map(|t| t.repo_root.as_deref().or(t.workspace_path.as_deref()))
        .ok_or_else(|| {
            OrbitError::InvalidInput(
                "merge_batch_pr: no task in batch has repo_root or workspace_path".to_string(),
            )
        })?;
    let repo_root = canonicalize_existing_dir(repo_root, "repo_root")?;

    // Get the current branch from the workspace
    let head = git_output(&repo_root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
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
    let _ = super::git::git_command_success(&repo_root, &["push", "origin", "--delete", &head]);

    let batch_requires_revision = batch_tasks.iter().any(task_required_revision);
    let batch_author = batch_tasks.iter().find_map(|task| {
        Some((
            task.actor_identity.agent_name()?.to_string(),
            task.actor_identity.agent_model()?.to_string(),
        ))
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
        && let Some((agent, model)) = batch_author
    {
        let _ = if batch_requires_revision {
            pr_scoreboard::record_pr_count_with_revision(host.scoreboard_dir(), &agent, &model)
        } else {
            pr_scoreboard::record_pr_count_without_revision(host.scoreboard_dir(), &agent, &model)
        };
    }

    Ok(json!({ "merged": true }))
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
    let base = input_string_field(input, "base").unwrap_or_else(|| "main".to_string());

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

    let mut completed_tasks = Vec::new();
    for task_id in &completed_task_ids {
        let task = host.get_task(task_id)?;
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
        let agent = task.actor_identity.agent_name()?;
        let model = task.actor_identity.agent_model().unwrap_or("unknown");
        Some(format!("*authored by: {agent} / {model}*"))
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
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Mutex;

    use chrono::Utc;
    use orbit_tools::ToolContext;
    use orbit_types::{
        Activity, ActorIdentity, Job, JobTargetType, OrbitError, OrbitEvent, ReviewMessage,
        ReviewThread, ReviewThreadStatus, Role, Task, TaskHistoryEntry, TaskPriority, TaskStatus,
        TaskType,
    };
    use serde_json::{Value, json};

    use super::{
        bootstrap_batch_review, build_batch_pr_body, merge_batch_pr, task_required_revision,
    };
    use crate::context::{JobRunResult, RuntimeHost, TaskAutomationUpdate, TaskHost};
    use crate::executor::automation::freshness::BranchFreshness;

    struct TestHost {
        tasks: Vec<Task>,
        merge_inputs: Mutex<Vec<Value>>,
        task_updates: Mutex<Vec<(String, TaskAutomationUpdate)>>,
        scoreboard_dir: PathBuf,
        scoring_enabled: bool,
    }

    impl TestHost {
        fn new(tasks: Vec<Task>) -> Self {
            Self {
                tasks,
                merge_inputs: Mutex::new(Vec::new()),
                task_updates: Mutex::new(Vec::new()),
                scoreboard_dir: PathBuf::from("."),
                scoring_enabled: false,
            }
        }

        fn with_scoreboard(tasks: Vec<Task>, scoreboard_dir: PathBuf) -> Self {
            Self {
                tasks,
                merge_inputs: Mutex::new(Vec::new()),
                task_updates: Mutex::new(Vec::new()),
                scoreboard_dir,
                scoring_enabled: true,
            }
        }
    }

    impl TaskHost for TestHost {
        fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
            self.tasks
                .iter()
                .find(|task| task.id == task_id)
                .cloned()
                .ok_or_else(|| OrbitError::TaskNotFound(task_id.to_string()))
        }

        fn list_tasks_filtered(
            &self,
            _status: Option<TaskStatus>,
            _priority: Option<TaskPriority>,
            _parent_id: Option<&str>,
            batch_id: Option<&str>,
        ) -> Result<Vec<Task>, OrbitError> {
            Ok(self
                .tasks
                .iter()
                .filter(|task| {
                    batch_id.is_none_or(|expected| task.batch_id.as_deref() == Some(expected))
                })
                .cloned()
                .collect())
        }

        fn start_task(
            &self,
            _task_id: &str,
            _note: Option<String>,
            _comment: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!("not used in merge_batch_pr tests")
        }

        fn update_task_from_activity(
            &self,
            _task_id: &str,
            _status: TaskStatus,
            _execution_summary: Option<String>,
            _comment: Option<String>,
            _note: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!("not used in merge_batch_pr tests")
        }

        fn apply_task_automation_update(
            &self,
            task_id: &str,
            update: TaskAutomationUpdate,
        ) -> Result<(), OrbitError> {
            self.task_updates
                .lock()
                .expect("task updates lock")
                .push((task_id.to_string(), update));
            Ok(())
        }
    }

    impl RuntimeHost for TestHost {
        fn record_event(&self, _event: OrbitEvent) -> Result<(), OrbitError> {
            Ok(())
        }

        fn repo_root(&self) -> Result<String, OrbitError> {
            Ok("/tmp".to_string())
        }

        fn data_root(&self) -> &Path {
            Path::new(".")
        }

        fn acquire_file_locks(
            &self,
            _task_id: &str,
            _repo_root: &str,
            _paths: &[&str],
        ) -> Result<(), OrbitError> {
            Ok(())
        }

        fn release_file_locks(&self, _task_id: &str) -> Result<usize, OrbitError> {
            Ok(0)
        }

        fn cleanup_stale_file_locks(&self) -> Result<usize, OrbitError> {
            Ok(0)
        }

        fn run_job_now_with_input_debug(
            &self,
            _job_id: &str,
            _input: Value,
            _debug: bool,
        ) -> Result<JobRunResult, OrbitError> {
            unimplemented!("not used in merge_batch_pr tests")
        }

        fn validate_activity_target_exists(
            &self,
            _target_type: JobTargetType,
            _target_id: &str,
        ) -> Result<Activity, OrbitError> {
            unimplemented!("not used in merge_batch_pr tests")
        }

        fn get_job(&self, _job_id: &str) -> Result<Option<Job>, OrbitError> {
            Ok(None)
        }

        fn run_tool_with_context_and_role(
            &self,
            name: &str,
            input: Value,
            role: Role,
            _tool_context: ToolContext,
        ) -> Result<Value, OrbitError> {
            assert_eq!(role, Role::Admin);
            assert_eq!(name, "github.pr.merge");
            self.merge_inputs
                .lock()
                .expect("merge input lock")
                .push(input);
            Ok(json!({ "merged": true }))
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
            self.scoring_enabled
        }

        fn scoreboard_dir(&self) -> &Path {
            &self.scoreboard_dir
        }
    }

    fn sample_task(id: &str, batch_id: &str, workspace_path: &str, pr_number: &str) -> Task {
        let now = Utc::now();
        Task {
            id: id.to_string(),
            parent_id: None,
            title: format!("Task {id}"),
            description: "test".to_string(),
            acceptance_criteria: vec![],
            plan: "plan".to_string(),
            execution_summary: String::new(),
            context_files: vec![],
            workspace_path: Some(workspace_path.to_string()),
            repo_root: Some(workspace_path.to_string()),
            assigned_to: None,
            created_by: None,
            actor_identity: ActorIdentity::default(),
            status: TaskStatus::Review,
            priority: TaskPriority::High,
            complexity: None,
            task_type: TaskType::Bug,
            pr_number: Some(pr_number.to_string()),
            pr_status: Some("APPROVED".to_string()),
            proposed_by: None,
            source_task_id: None,
            batch_id: Some(batch_id.to_string()),
            comments: vec![],
            history: vec![],
            review_threads: vec![],
            created_at: now,
            updated_at: now,
        }
    }

    fn sample_task_for_pr(id: &str, workspace_path: &str, pr_number: &str) -> Task {
        let mut task = sample_task(id, "batch-1", workspace_path, pr_number);
        task.batch_id = None;
        task
    }

    fn scoreboard_value(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path.join("pr.json")).expect("pr.json"))
            .expect("valid scoreboard json")
    }

    fn init_batch_merge_repo() -> tempfile::TempDir {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo_root = tempdir.path();

        run_git(
            repo_root,
            &["init", "--initial-branch=orbit/parallel-batch"],
        );
        run_git(repo_root, &["config", "user.name", "Orbit Tests"]);
        run_git(
            repo_root,
            &["config", "user.email", "orbit-tests@example.com"],
        );
        std::fs::write(repo_root.join("README.md"), "batch\n").expect("write readme");
        run_git(repo_root, &["add", "README.md"]);
        run_git(repo_root, &["commit", "-m", "initial"]);
        run_git(repo_root, &["branch", "main"]);

        tempdir
    }

    fn run_git(repo_root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(repo_root)
            .status()
            .expect("run git");
        assert!(status.success(), "git {:?} failed", args);
    }

    #[test]
    fn merge_batch_pr_disables_branch_deletion_for_shared_worktree_merges() {
        let repo = init_batch_merge_repo();
        let repo_root = repo.path().to_string_lossy().to_string();
        let host = TestHost::new(vec![sample_task(
            "T20260330-063823",
            "batch-1",
            &repo_root,
            "76",
        )]);

        let result = merge_batch_pr(
            &host,
            &json!({
                "run_id": "batch-1",
                "base": "main",
            }),
        )
        .expect("merge_batch_pr succeeds");

        assert_eq!(result, json!({ "merged": true }));
        let merge_inputs = host.merge_inputs.lock().expect("merge inputs");
        assert_eq!(merge_inputs.len(), 1);
        assert_eq!(
            merge_inputs[0],
            json!({
                "pr": "76",
                "strategy": "squash",
                "delete_branch": false,
            })
        );

        let updated = host.task_updates.lock().expect("task updates");
        assert_eq!(
            updated
                .iter()
                .map(|(task_id, update)| (task_id.clone(), update.status))
                .collect::<Vec<_>>()
                .as_slice(),
            &[("T20260330-063823".to_string(), Some(TaskStatus::Done))]
        );
    }

    #[test]
    fn bootstrap_batch_review_is_noop_when_batch_already_exists() {
        let host = TestHost::new(vec![sample_task("T20260330-063823", "batch-9", "/tmp", "76")]);

        let result = bootstrap_batch_review(
            &host,
            &json!({
                "run_id": "batch-9",
            }),
        )
        .expect("bootstrap succeeds");

        assert_eq!(
            result,
            json!({
                "batch_id": "batch-9",
                "task_count": 1,
                "tagged_count": 0,
            })
        );
        assert!(host.task_updates.lock().expect("task updates").is_empty());
    }

    #[test]
    fn bootstrap_batch_review_tags_tasks_by_pr_number() {
        let host = TestHost::new(vec![
            sample_task_for_pr("T20260330-063823", "/tmp", "76"),
            sample_task_for_pr("T20260330-065846", "/tmp", "76"),
            sample_task_for_pr("T20260330-071500", "/tmp", "77"),
        ]);

        let result = bootstrap_batch_review(
            &host,
            &json!({
                "run_id": "batch-9",
                "pr_number": "76",
            }),
        )
        .expect("bootstrap succeeds");

        assert_eq!(
            result,
            json!({
                "batch_id": "batch-9",
                "task_count": 2,
                "tagged_count": 2,
            })
        );

        let updated = host.task_updates.lock().expect("task updates");
        assert_eq!(
            updated
                .iter()
                .map(|(task_id, update)| (task_id.clone(), update.batch_id.clone()))
                .collect::<Vec<_>>(),
            vec![
                ("T20260330-063823".to_string(), Some("batch-9".to_string())),
                ("T20260330-065846".to_string(), Some("batch-9".to_string())),
            ]
        );
    }

    #[test]
    fn bootstrap_batch_review_errors_when_no_tasks_match_pr() {
        let host = TestHost::new(vec![sample_task_for_pr("T20260330-071500", "/tmp", "77")]);

        let error = bootstrap_batch_review(
            &host,
            &json!({
                "run_id": "batch-9",
                "pr_number": "76",
            }),
        )
        .expect_err("bootstrap should fail");

        assert!(
            error
                .to_string()
                .contains("bootstrap_batch_review: no tasks found for pr_number '76'")
        );
    }

    #[test]
    fn build_batch_pr_body_includes_execution_summaries_and_signature() {
        let mut first = sample_task("T20260330-063823", "batch-1", "/tmp", "76");
        first.execution_summary = "## Status\nsuccess".to_string();
        first.actor_identity = ActorIdentity::agent("codex", "gpt-5.4");

        let second = sample_task("T20260330-065846", "batch-1", "/tmp", "76");
        let freshness = BranchFreshness {
            base_ref: "origin/main".to_string(),
            head_ref: "orbit/parallel-batch".to_string(),
            commits_behind: 0,
            commits_ahead: 3,
        };
        let body = build_batch_pr_body(
            &[first, second],
            &freshness,
            &["orbit/orbit-engine/src/executor/automation/pr.rs"],
        );

        assert!(body.contains("### T20260330-063823: Task T20260330-063823"));
        assert!(body.contains("## Status\nsuccess"));
        assert_eq!(
            body.matches("<details><summary>Execution Summary</summary>")
                .count(),
            1
        );
        assert!(body.contains("### T20260330-065846: Task T20260330-065846"));
        assert!(body.ends_with("*authored by: codex / gpt-5.4*"));
    }

    #[test]
    fn merge_batch_pr_records_one_without_revision_metric_per_batch_author() {
        let repo = init_batch_merge_repo();
        let repo_root = repo.path().to_string_lossy().to_string();
        let scoreboard_dir = tempfile::tempdir().expect("scoreboard dir");

        let mut first = sample_task("T20260330-063823", "batch-1", &repo_root, "76");
        first.actor_identity = ActorIdentity::agent("codex", "gpt-5.4");
        let mut second = sample_task("T20260330-065846", "batch-1", &repo_root, "76");
        second.actor_identity = ActorIdentity::agent("codex", "gpt-5.4");

        let host =
            TestHost::with_scoreboard(vec![first, second], scoreboard_dir.path().to_path_buf());

        merge_batch_pr(
            &host,
            &json!({
                "run_id": "batch-1",
                "base": "main",
            }),
        )
        .expect("merge_batch_pr succeeds");

        let scoreboard = scoreboard_value(scoreboard_dir.path());
        assert_eq!(
            scoreboard["pr-count-without-revision"]["codex"]["gpt-5.4"],
            1
        );
        assert!(scoreboard.get("pr-count-with-revision").is_none());
    }

    #[test]
    fn merge_batch_pr_records_revision_metric_when_review_threads_were_resolved() {
        let repo = init_batch_merge_repo();
        let repo_root = repo.path().to_string_lossy().to_string();
        let scoreboard_dir = tempfile::tempdir().expect("scoreboard dir");

        let mut task = sample_task("T20260330-063823", "batch-1", &repo_root, "76");
        task.actor_identity = ActorIdentity::agent("codex", "gpt-5.4");
        task.review_threads = vec![ReviewThread {
            thread_id: "rt-1".to_string(),
            path: Some("orbit/orbit-engine/src/executor/automation/pr.rs".to_string()),
            line: Some(42),
            status: ReviewThreadStatus::Resolved,
            messages: vec![ReviewMessage {
                message_id: "rm-1".to_string(),
                at: Utc::now(),
                by: "claude / sonnet".to_string(),
                body: "Please fix this.".to_string(),
                github_comment_id: Some(10),
            }],
            github_thread_id: Some(10),
        }];

        let host = TestHost::with_scoreboard(vec![task], scoreboard_dir.path().to_path_buf());

        merge_batch_pr(
            &host,
            &json!({
                "run_id": "batch-1",
                "base": "main",
            }),
        )
        .expect("merge_batch_pr succeeds");

        let scoreboard = scoreboard_value(scoreboard_dir.path());
        assert_eq!(scoreboard["pr-count-with-revision"]["codex"]["gpt-5.4"], 1);
        assert!(scoreboard.get("pr-count-without-revision").is_none());
    }

    #[test]
    fn task_required_revision_detects_review_rejection_history() {
        let mut task = sample_task("T20260330-063823", "batch-1", "/tmp", "76");
        task.history.push(TaskHistoryEntry {
            at: Utc::now(),
            by: "human".to_string(),
            event: "status_changed".to_string(),
            note: Some("needs changes".to_string()),
            from_status: Some(TaskStatus::Review),
            to_status: Some(TaskStatus::Rejected),
        });

        assert!(task_required_revision(&task));
    }
}
