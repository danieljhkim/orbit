use std::path::Path;
use std::process::Command;

use orbit_types::OrbitError;
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskHost};

use super::git::{git_output, git_output_paths, git_success};
use super::input::{canonicalize_existing_dir, input_string_field};

pub(super) fn commit_batch_changes<H: TaskHost + RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let workspace_path = match input_string_field(input, "workspace_path") {
        Some(ws) => canonicalize_existing_dir(&ws, "workspace_path")?,
        None => {
            let repo_root_str = host.repo_root()?;
            let repo_root = Path::new(&repo_root_str);
            super::parallel::resolve_shared_worktree_path(repo_root)?
        }
    };

    let actual_branch = git_output(&workspace_path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let actual_branch = actual_branch.trim();
    if actual_branch == "HEAD" {
        return Err(OrbitError::Execution(format!(
            "workspace '{}' has detached HEAD; expected a named branch",
            workspace_path.display(),
        )));
    }

    let batch_id = input
        .get("run_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            OrbitError::InvalidInput("commit_batch_changes requires input.run_id".to_string())
        })?;

    let batch_tasks = host.list_tasks_filtered(None, None, None, Some(batch_id))?;
    let completed_task_ids: Vec<String> = batch_tasks.iter().map(|t| t.id.clone()).collect();

    if completed_task_ids.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "commit_batch_changes: no tasks found for batch_id '{batch_id}'"
        )));
    }

    ensure_no_unmerged_changes(&workspace_path)?;
    run_cargo_fmt(&workspace_path)?;
    git_success(&workspace_path, &["add", "--all", "--", "."])?;

    let changed_files = git_output_paths(
        &workspace_path,
        &["diff", "--cached", "--name-only", "-z", "--relative"],
    )?;

    if changed_files.is_empty() {
        git_success(&workspace_path, &["reset", "HEAD"])?;
        return Ok(json!({}));
    }

    let mut task_lines = Vec::new();
    let mut id_labels = Vec::new();
    for task_id in &completed_task_ids {
        let task = host.get_task(task_id)?;
        task_lines.push(format!("- {}: {}", task_id, task.title.trim()));
        id_labels.push(task_id.clone());
    }
    let ids_joined = id_labels.join(", ");
    let message = format!(
        "feat: parallel batch [{}]\n\nTasks:\n{}",
        ids_joined,
        task_lines.join("\n")
    );

    git_success(&workspace_path, &["commit", "-m", &message])?;
    Ok(json!({}))
}

/// Runs `cargo fmt --all` in the given directory.
/// Extracted so tests can verify the formatting step independently.
fn run_cargo_fmt(workspace_path: &Path) -> Result<(), OrbitError> {
    let fmt_output = Command::new("cargo")
        .args(["fmt", "--all"])
        .current_dir(workspace_path)
        .output()
        .map_err(|e| OrbitError::Execution(format!("failed to spawn cargo fmt: {e}")))?;

    if !fmt_output.status.success() {
        let stdout = String::from_utf8_lossy(&fmt_output.stdout);
        let stderr = String::from_utf8_lossy(&fmt_output.stderr);
        let exit_code = fmt_output.status.code().unwrap_or(1);
        return Err(OrbitError::Execution(format!(
            "cargo fmt failed before commit (exit_code={exit_code})\nstdout:\n{stdout}\nstderr:\n{stderr}"
        )));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::TaskAutomationUpdate;
    use orbit_types::{
        Activity, Job, JobTargetType, OrbitError, OrbitEvent, Role, Task, TaskPriority, TaskStatus,
        TaskType,
    };
    use orbit_tools::ToolContext;
    use serde_json::{Value, json};
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// Minimal mock implementing TaskHost + RuntimeHost for commit tests.
    struct CommitTestHost {
        tasks: Vec<Task>,
    }

    impl CommitTestHost {
        fn with_tasks(tasks: Vec<Task>) -> Self {
            Self { tasks }
        }
    }

    impl TaskHost for CommitTestHost {
        fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
            self.tasks
                .iter()
                .find(|t| t.id == task_id)
                .cloned()
                .ok_or_else(|| OrbitError::TaskNotFound(format!("task {task_id}")))
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
                .filter(|t| batch_id.is_none() || t.batch_id.as_deref() == batch_id)
                .cloned()
                .collect())
        }

        fn start_task(
            &self,
            _task_id: &str,
            _note: Option<String>,
            _comment: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!()
        }

        fn update_task_from_activity(
            &self,
            _task_id: &str,
            _status: TaskStatus,
            _execution_summary: Option<String>,
            _comment: Option<String>,
            _note: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!()
        }

        fn apply_task_automation_update(
            &self,
            _task_id: &str,
            _update: TaskAutomationUpdate,
        ) -> Result<(), OrbitError> {
            unimplemented!()
        }
    }

    impl RuntimeHost for CommitTestHost {
        fn record_event(&self, _event: OrbitEvent) -> Result<(), OrbitError> {
            Ok(())
        }
        fn repo_root(&self) -> Result<String, OrbitError> {
            unimplemented!()
        }
        fn data_root(&self) -> &Path {
            unimplemented!()
        }
        fn acquire_file_locks(
            &self,
            _task_id: &str,
            _repo_root: &str,
            _paths: &[&str],
        ) -> Result<(), OrbitError> {
            unimplemented!()
        }
        fn release_file_locks(&self, _task_id: &str) -> Result<usize, OrbitError> {
            unimplemented!()
        }
        fn cleanup_stale_file_locks(&self) -> Result<usize, OrbitError> {
            unimplemented!()
        }
        fn run_job_now_with_input_debug(
            &self,
            _job_id: &str,
            _input: Value,
            _debug: bool,
        ) -> Result<crate::context::JobRunResult, OrbitError> {
            unimplemented!()
        }
        fn validate_activity_target_exists(
            &self,
            _target_type: JobTargetType,
            _target_id: &str,
        ) -> Result<Activity, OrbitError> {
            unimplemented!()
        }
        fn get_job(&self, _job_id: &str) -> Result<Option<Job>, OrbitError> {
            unimplemented!()
        }
        fn run_tool_with_context_and_role(
            &self,
            _name: &str,
            _input: Value,
            _role: Role,
            _tool_context: ToolContext,
        ) -> Result<Value, OrbitError> {
            unimplemented!()
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
            unimplemented!()
        }
        fn scoring_enabled(&self) -> bool {
            false
        }
        fn scoreboard_dir(&self) -> &Path {
            Path::new("/tmp")
        }
    }

    fn make_task(id: &str, title: &str, batch_id: &str) -> Task {
        Task {
            id: id.to_string(),
            parent_id: None,
            title: title.to_string(),
            description: String::new(),
            acceptance_criteria: vec![],
            plan: String::new(),
            execution_summary: String::new(),
            context_files: vec![],
            workspace_path: None,
            repo_root: None,
            assigned_to: None,
            created_by: None,
            actor_identity: Default::default(),
            status: TaskStatus::InProgress,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Chore,
            pr_number: None,
            pr_status: None,
            proposed_by: None,
            source_task_id: None,
            batch_id: Some(batch_id.to_string()),
            comments: vec![],
            history: vec![],
            review_threads: vec![],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    /// Creates a temp directory with a minimal Cargo project and git repo.
    fn setup_cargo_git_repo() -> (TempDir, PathBuf) {
        let tmp = TempDir::new().expect("create temp dir");
        let dir = tmp.path().to_path_buf();

        std::process::Command::new("git")
            .args(["init", "-b", "test-branch"])
            .current_dir(&dir)
            .output()
            .expect("git init");
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&dir)
            .output()
            .expect("git config email");
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&dir)
            .output()
            .expect("git config name");

        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"test-proj\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("write Cargo.toml");
        std::fs::create_dir_all(dir.join("src")).expect("create src dir");
        std::fs::write(dir.join("src/lib.rs"), "pub fn hello() {}\n").expect("write lib.rs");

        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&dir)
            .output()
            .expect("git add");
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(&dir)
            .output()
            .expect("git commit");

        (tmp, dir)
    }

    #[test]
    fn run_cargo_fmt_succeeds_on_valid_cargo_project() {
        let (_tmp, dir) = setup_cargo_git_repo();
        std::fs::write(dir.join("src/lib.rs"), "pub fn hello(){let     x=1;}\n")
            .expect("write unformatted code");

        let result = run_cargo_fmt(&dir);
        assert!(result.is_ok(), "cargo fmt should succeed: {result:?}");

        let contents = std::fs::read_to_string(dir.join("src/lib.rs")).expect("read lib.rs");
        assert!(
            !contents.contains("let     x"),
            "file should be formatted after cargo fmt"
        );
    }

    #[test]
    fn run_cargo_fmt_fails_without_cargo_toml() {
        let tmp = TempDir::new().expect("create temp dir");
        let dir = tmp.path();

        let result = run_cargo_fmt(dir);
        assert!(result.is_err(), "cargo fmt should fail without Cargo.toml");

        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("cargo fmt failed"),
            "error should mention cargo fmt failure: {err_msg}"
        );
    }

    #[test]
    fn commit_batch_changes_formats_before_staging() {
        let (_tmp, dir) = setup_cargo_git_repo();
        let host = CommitTestHost::with_tasks(vec![make_task("T-001", "Test task", "batch-1")]);

        std::fs::write(dir.join("src/lib.rs"), "pub fn hello(){let     x=1;}\n")
            .expect("write unformatted code");

        let input = json!({
            "workspace_path": dir.to_str().unwrap(),
            "run_id": "batch-1"
        });

        let result = commit_batch_changes(&host, &input);
        assert!(result.is_ok(), "commit should succeed: {result:?}");

        let committed = std::process::Command::new("git")
            .args(["show", "HEAD:src/lib.rs"])
            .current_dir(&dir)
            .output()
            .expect("git show");
        let committed_content = String::from_utf8_lossy(&committed.stdout);
        assert!(
            !committed_content.contains("let     x"),
            "committed file should be formatted"
        );
    }

    #[test]
    fn commit_batch_changes_aborts_on_fmt_failure() {
        let tmp = TempDir::new().expect("create temp dir");
        let dir = tmp.path();

        std::process::Command::new("git")
            .args(["init", "-b", "test-branch"])
            .current_dir(dir)
            .output()
            .expect("git init");
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir)
            .output()
            .expect("git config email");
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir)
            .output()
            .expect("git config name");

        std::fs::write(dir.join("hello.txt"), "hello\n").expect("write file");
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .output()
            .expect("git add");
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(dir)
            .output()
            .expect("git commit");

        std::fs::write(dir.join("new.txt"), "new\n").expect("write new file");

        let host = CommitTestHost::with_tasks(vec![make_task("T-002", "Another task", "batch-2")]);

        let input = json!({
            "workspace_path": dir.to_str().unwrap(),
            "run_id": "batch-2"
        });

        let result = commit_batch_changes(&host, &input);
        assert!(result.is_err(), "commit should fail when cargo fmt fails");

        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("cargo fmt failed"),
            "error should mention cargo fmt: {err_msg}"
        );

        // Verify no commit was made
        let log = std::process::Command::new("git")
            .args(["log", "--oneline"])
            .current_dir(dir)
            .output()
            .expect("git log");
        let log_str = String::from_utf8_lossy(&log.stdout);
        assert!(
            log_str.contains("initial") && !log_str.contains("parallel batch"),
            "no commit should have been created after fmt failure"
        );
    }
}
