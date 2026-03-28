use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_store::pr_scoreboard;
use orbit_types::OrbitError;
use serde_json::{Value, json};

use super::input::required_input_string;
use crate::context::{RuntimeHost, TaskHost};

const TIMEOUT_MS: u64 = 15_000;

pub(super) fn load_pr_comments<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let task = host.get_task(task_id)?;
    let pr_number = task.pr_number.as_deref().ok_or_else(|| {
        OrbitError::InvalidInput("load_pr_comments requires task.pr_number".to_string())
    })?;

    let repo_root = task
        .repo_root
        .as_deref()
        .or(task.workspace_path.as_deref())
        .ok_or_else(|| {
            OrbitError::InvalidInput(
                "load_pr_comments requires task.repo_root or task.workspace_path".to_string(),
            )
        })?;

    let comments = fetch_pr_review_comments(repo_root, pr_number)?;

    // Filter to unresolved comments (not part of a resolved thread).
    // GitHub's REST API doesn't directly flag "resolved" on individual comments,
    // but we can use the review threads endpoint to check.
    let unresolved = filter_unresolved_comments(repo_root, pr_number, &comments)?;

    if unresolved.is_empty() {
        return Ok(json!({
            "loop_exit": true,
            "comments": [],
            "comment_summary": "No unresolved comments.",
        }));
    }

    if host.scoring_enabled()
        && let (Some(agent), Some(model)) = (
            task.actor_identity.agent_name(),
            task.actor_identity.agent_model(),
        )
    {
        let _ = pr_scoreboard::record_pr_revision(host.scoreboard_dir(), agent, model);
    }

    let summary = build_comment_summary(&unresolved);
    Ok(json!({
        "loop_exit": false,
        "comments": unresolved,
        "comment_summary": summary,
    }))
}

fn fetch_pr_review_comments(repo_root: &str, pr_number: &str) -> Result<Vec<Value>, OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "gh".to_string(),
            args: vec![
                "api".to_string(),
                format!("repos/{{owner}}/{{repo}}/pulls/{pr_number}/comments"),
                "--paginate".to_string(),
            ],
            current_dir: Some(repo_root.to_string()),
            timeout_ms: Some(TIMEOUT_MS),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        return Err(OrbitError::Execution(format!(
            "failed to fetch PR comments for '{}': {}",
            pr_number,
            result.stderr.trim()
        )));
    }

    let comments: Vec<Value> = serde_json::from_str(result.stdout.trim()).unwrap_or_default();
    Ok(comments)
}

fn filter_unresolved_comments(
    repo_root: &str,
    pr_number: &str,
    comments: &[Value],
) -> Result<Vec<Value>, OrbitError> {
    // Fetch review threads to determine which are resolved.
    // Each thread has `isResolved` and a `comments` array whose entries
    // carry a `databaseId` that matches the REST API comment `id`.
    let result = run_process(
        &ExecRequest {
            program: "gh".to_string(),
            args: vec![
                "pr".to_string(),
                "view".to_string(),
                pr_number.to_string(),
                "--json".to_string(),
                "reviewThreads".to_string(),
            ],
            current_dir: Some(repo_root.to_string()),
            timeout_ms: Some(TIMEOUT_MS),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        // If we can't fetch threads, return all comments as unresolved
        // (conservative approach).
        return Ok(comments.to_vec());
    }

    let payload: Value = serde_json::from_str(result.stdout.trim()).unwrap_or_default();

    // Collect databaseIds of all comments that belong to resolved threads.
    let resolved_comment_ids: std::collections::HashSet<u64> = payload
        .get("reviewThreads")
        .and_then(Value::as_array)
        .map(|threads| {
            threads
                .iter()
                .filter(|t| {
                    t.get("isResolved")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                })
                .flat_map(|t| {
                    t.get("comments")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|c| c.get("databaseId").and_then(Value::as_u64))
                })
                .collect()
        })
        .unwrap_or_default();

    if resolved_comment_ids.is_empty() {
        return Ok(comments.to_vec());
    }

    // Keep only comments whose REST API `id` is NOT in a resolved thread.
    let unresolved: Vec<Value> = comments
        .iter()
        .filter(|c| {
            let id = c.get("id").and_then(Value::as_u64).unwrap_or(0);
            !resolved_comment_ids.contains(&id)
        })
        .cloned()
        .collect();

    Ok(unresolved)
}

/// Read and parse the PR scoreboard file from `repo_root/.orbit/scoreboard/pr.json`.
#[cfg(test)]
fn read_pr_scoreboard(
    repo_root: &std::path::Path,
) -> Option<
    std::collections::HashMap<
        String,
        std::collections::HashMap<String, std::collections::HashMap<String, u64>>,
    >,
> {
    let path = repo_root.join(".orbit/scoreboard/pr.json");
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

fn build_comment_summary(comments: &[Value]) -> String {
    let mut summary = format!("{} unresolved comment(s):\n", comments.len());
    for (i, comment) in comments.iter().enumerate() {
        let path = comment
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let line = comment
            .get("line")
            .or_else(|| comment.get("original_line"))
            .and_then(Value::as_u64)
            .map(|n| n.to_string())
            .unwrap_or_else(|| "?".to_string());
        let body = comment
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(200)
            .collect::<String>();
        let user = comment
            .get("user")
            .and_then(|u| u.get("login"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        summary.push_str(&format!(
            "\n{}. {} ({}:{}) — {}\n",
            i + 1,
            user,
            path,
            line,
            body
        ));
    }
    summary
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::path::Path;
    use std::sync::Mutex;

    use chrono::Utc;
    use orbit_tools::ToolContext;
    use orbit_types::{
        Activity, ActorIdentity, JobTargetType, OrbitError, OrbitEvent, Role, Task, TaskPriority,
        TaskStatus, TaskType,
    };
    use serde_json::{Value, json};
    use tempfile::TempDir;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

    struct FakeHost {
        task: RefCell<Option<Task>>,
        scoring_enabled: bool,
        scoreboard_dir: std::path::PathBuf,
    }

    impl FakeHost {
        fn new(task: Task) -> Self {
            let scoreboard_dir = task
                .repo_root
                .as_deref()
                .or(task.workspace_path.as_deref())
                .map(|p| std::path::Path::new(p).join(".orbit").join("scoreboard"))
                .unwrap_or_default();
            Self {
                task: RefCell::new(Some(task)),
                scoring_enabled: false,
                scoreboard_dir,
            }
        }

        fn with_scoring(mut self) -> Self {
            self.scoring_enabled = true;
            self
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
            Ok(())
        }
    }

    impl RuntimeHost for FakeHost {
        fn record_event(&self, _event: OrbitEvent) -> Result<(), OrbitError> {
            Ok(())
        }

        fn repo_root(&self) -> Result<String, OrbitError> {
            Err(OrbitError::Execution("not used".to_string()))
        }

        fn data_root(&self) -> &std::path::Path {
            std::path::Path::new(".")
        }

        fn validate_activity_target_exists(
            &self,
            _target_type: JobTargetType,
            _target_id: &str,
        ) -> Result<Activity, OrbitError> {
            unimplemented!()
        }

        fn get_job(&self, _job_id: &str) -> Result<Option<orbit_types::Job>, OrbitError> {
            Ok(None)
        }

        fn run_tool_with_context_and_role(
            &self,
            _name: &str,
            _input: Value,
            _role: Role,
            _tool_context: ToolContext,
        ) -> Result<Value, OrbitError> {
            Ok(json!({}))
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

        fn scoreboard_dir(&self) -> &std::path::Path {
            &self.scoreboard_dir
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
        crate::executor::automation::test_utils::path_lock()
    }

    fn prepend_path(dir: &Path) -> String {
        let mut entries = vec![dir.to_string_lossy().to_string()];
        if let Some(existing) = std::env::var_os("PATH") {
            entries.push(existing.to_string_lossy().to_string());
        }
        entries.join(":")
    }

    /// Install a fake `gh` script that responds to:
    /// - `gh api repos/{owner}/{repo}/pulls/{pr}/comments --paginate` → `comments_json`
    /// - `gh pr view {pr} --json reviewThreads` → `threads_json`
    fn install_fake_gh(bin_dir: &Path, comments_json: &str, threads_json: &str) {
        let script = format!(
            concat!(
                "#!/bin/sh\n",
                "if [ \"$1\" = \"api\" ]; then\n",
                "  printf '%s' '{comments}'\n",
                "  exit 0\n",
                "fi\n",
                "if [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ] && [ \"$4\" = \"--json\" ] && [ \"$5\" = \"reviewThreads\" ]; then\n",
                "  printf '%s' '{threads}'\n",
                "  exit 0\n",
                "fi\n",
                "printf '%s\\n' \"unexpected gh args: $*\" >&2\n",
                "exit 1\n"
            ),
            comments = comments_json.replace('\'', "'\\''"),
            threads = threads_json.replace('\'', "'\\''"),
        );
        let gh_path = bin_dir.join("gh");
        fs::write(&gh_path, script).expect("write fake gh");
        #[cfg(unix)]
        fs::set_permissions(&gh_path, fs::Permissions::from_mode(0o755)).expect("chmod gh");
    }

    fn use_fake_gh(comments_json: &str, threads_json: &str) -> (TempDir, PathGuard) {
        let bin_dir = tempfile::tempdir().expect("temp gh dir");
        install_fake_gh(bin_dir.path(), comments_json, threads_json);
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

    fn test_task(repo_root: &Path) -> Task {
        Task {
            id: "T20260320-021158".to_string(),
            parent_id: None,
            title: "test task".to_string(),
            description: "desc".to_string(),
            plan: "plan".to_string(),
            execution_summary: String::new(),
            context_files: vec![],
            workspace_path: Some(repo_root.to_string_lossy().to_string()),
            repo_root: Some(repo_root.to_string_lossy().to_string()),
            assigned_to: None,
            created_by: Some("test".to_string()),
            actor_identity: ActorIdentity::agent("claude", "opus-4.6"),
            status: TaskStatus::Review,
            priority: TaskPriority::High,
            task_type: TaskType::Issue,
            pr_number: Some("42".to_string()),
            pr_status: None,
            proposed_by: None,
            source_task_id: None,
            complexity: None,
            comments: vec![],
            history: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn load_pr_comments_records_scoreboard_for_unresolved_comments() {
        let repo_dir = tempfile::tempdir().expect("temp dir");
        let comments_json = serde_json::to_string(&json!([
            {"id": 1, "body": "fix this", "path": "src/main.rs", "line": 10, "user": {"login": "reviewer"}},
            {"id": 2, "body": "and this", "path": "src/lib.rs", "line": 20, "user": {"login": "reviewer"}},
        ]))
        .unwrap();
        let threads_json = serde_json::to_string(&json!({
            "reviewThreads": [
                {"isResolved": false, "comments": [{"databaseId": 1}]},
                {"isResolved": false, "comments": [{"databaseId": 2}]},
            ]
        }))
        .unwrap();

        let (_gh_dir, _path_guard) = use_fake_gh(&comments_json, &threads_json);
        let host = FakeHost::new(test_task(repo_dir.path())).with_scoring();

        let result = load_pr_comments(&host, &json!({"task_id": "T20260320-021158"}))
            .expect("load_pr_comments should succeed");

        assert_eq!(result["loop_exit"], json!(false));
        assert_eq!(result["comments"].as_array().unwrap().len(), 2);

        let sb = read_pr_scoreboard(repo_dir.path()).expect("scoreboard should exist");
        assert_eq!(sb["revisions"]["claude"]["opus-4.6"], 1);
    }

    #[test]
    fn load_pr_comments_no_scoreboard_when_all_resolved() {
        let repo_dir = tempfile::tempdir().expect("temp dir");
        let comments_json = serde_json::to_string(&json!([
            {"id": 1, "body": "fix this", "path": "src/main.rs", "line": 10, "user": {"login": "reviewer"}},
        ]))
        .unwrap();
        let threads_json = serde_json::to_string(&json!({
            "reviewThreads": [
                {"isResolved": true, "comments": [{"databaseId": 1}]},
            ]
        }))
        .unwrap();

        let (_gh_dir, _path_guard) = use_fake_gh(&comments_json, &threads_json);
        let host = FakeHost::new(test_task(repo_dir.path()));

        let result = load_pr_comments(&host, &json!({"task_id": "T20260320-021158"}))
            .expect("load_pr_comments should succeed");

        assert_eq!(result["loop_exit"], json!(true));
        assert_eq!(result["comments"].as_array().unwrap().len(), 0);

        // No scoreboard should be created when there are no unresolved comments
        let sb = read_pr_scoreboard(repo_dir.path());
        assert!(
            sb.is_none(),
            "scoreboard should not exist when no unresolved comments"
        );
    }

    #[test]
    fn load_pr_comments_skips_scoreboard_when_agent_missing() {
        let repo_dir = tempfile::tempdir().expect("temp dir");
        let comments_json = serde_json::to_string(&json!([
            {"id": 1, "body": "fix this", "path": "src/main.rs", "line": 10, "user": {"login": "reviewer"}},
        ]))
        .unwrap();
        let threads_json = serde_json::to_string(&json!({
            "reviewThreads": [
                {"isResolved": false, "comments": [{"databaseId": 1}]},
            ]
        }))
        .unwrap();

        let (_gh_dir, _path_guard) = use_fake_gh(&comments_json, &threads_json);
        let mut task = test_task(repo_dir.path());
        task.actor_identity = ActorIdentity::System;
        let host = FakeHost::new(task);

        let result = load_pr_comments(&host, &json!({"task_id": "T20260320-021158"}))
            .expect("load_pr_comments should succeed");

        assert_eq!(result["loop_exit"], json!(false));

        let sb = read_pr_scoreboard(repo_dir.path());
        assert!(
            sb.is_none(),
            "scoreboard should not exist when agent/model missing"
        );
    }
}
