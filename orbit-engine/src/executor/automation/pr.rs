use orbit_store::pr_scoreboard;
use orbit_tools::ToolContext;
use orbit_types::{OrbitError, Role, Task, TaskStatus};
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

use super::freshness::ensure_branch_fresh_against_base;
use super::git::git_output;
use super::input::{
    canonicalize_existing_dir, input_string_field, json_number_to_string, required_input_string,
};

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
    let base = input_string_field(input, "base").unwrap_or_else(|| "agent-main".to_string());

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
            // Parallel batch branches stay attached to the shared worktree
            // until cleanup, so deleting the local branch here can fail even
            // after the PR merge itself succeeded.
            "delete_branch": false,
        }),
        Role::Admin,
        tool_context,
    )?;

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

        // Record PR merge to scoreboard for each task's actor identity
        if host.scoring_enabled()
            && let (Some(agent), Some(model)) = (
                task.actor_identity.agent_name(),
                task.actor_identity.agent_model(),
            )
        {
            let _ = pr_scoreboard::record_pr_merged(host.scoreboard_dir(), agent, model);
        }
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Mutex;

    use chrono::Utc;
    use orbit_tools::ToolContext;
    use orbit_types::{
        Activity, ActorIdentity, Job, JobTargetType, OrbitError, OrbitEvent, Role, Task,
        TaskPriority, TaskStatus, TaskType,
    };
    use serde_json::{Value, json};

    use super::{build_batch_pr_body, merge_batch_pr};
    use crate::context::{JobRunResult, RuntimeHost, TaskAutomationUpdate, TaskHost};
    use crate::executor::automation::freshness::BranchFreshness;

    struct TestHost {
        tasks: Vec<Task>,
        merge_inputs: Mutex<Vec<Value>>,
        updated_statuses: Mutex<Vec<(String, Option<TaskStatus>)>>,
        scoreboard_dir: PathBuf,
    }

    impl TestHost {
        fn new(tasks: Vec<Task>) -> Self {
            Self {
                tasks,
                merge_inputs: Mutex::new(Vec::new()),
                updated_statuses: Mutex::new(Vec::new()),
                scoreboard_dir: PathBuf::from("."),
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
            self.updated_statuses
                .lock()
                .expect("updated statuses lock")
                .push((task_id.to_string(), update.status));
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
            false
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
        run_git(repo_root, &["branch", "agent-main"]);

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
                "base": "agent-main",
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

        let updated = host.updated_statuses.lock().expect("updated statuses");
        assert_eq!(
            updated.as_slice(),
            &[("T20260330-063823".to_string(), Some(TaskStatus::Done))]
        );
    }

    #[test]
    fn build_batch_pr_body_includes_execution_summaries_and_signature() {
        let mut first = sample_task("T20260330-063823", "batch-1", "/tmp", "76");
        first.execution_summary = "## Status\nsuccess".to_string();
        first.actor_identity = ActorIdentity::agent("codex", "gpt-5.4");

        let second = sample_task("T20260330-065846", "batch-1", "/tmp", "76");
        let freshness = BranchFreshness {
            base_ref: "origin/agent-main".to_string(),
            head_ref: "orbit/parallel-batch".to_string(),
            commits_behind: 0,
            commits_ahead: 3,
        };
        let body = build_batch_pr_body(
            &[first, second],
            &freshness,
            &["orbit-engine/src/executor/automation/pr.rs"],
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
}
