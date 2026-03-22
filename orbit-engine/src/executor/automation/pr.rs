use orbit_tools::ToolContext;
use orbit_types::{OrbitError, Role, TaskStatus};
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

use super::freshness::ensure_branch_fresh_against_base;
use super::git::git_output;
use super::input::{
    canonicalize_existing_dir, input_string_field, json_number_to_string, required_input_string,
};
use super::review::resolve_review_decision;

pub(super) fn merge_pr_from_task<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let task = host.get_task(task_id)?;
    let repo_root = canonicalize_existing_dir(
        &task
            .repo_root
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
    let review_decision = resolve_review_decision(&repo_root, pr_number)?;
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
        &task
            .repo_root
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

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use orbit_types::TaskType;

    use super::*;
    use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

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
            unimplemented!("validate_activity_target_exists is not used in automation merge tests")
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

        fn maybe_create_failure_task(
            &self,
            _job_id: &str,
            _run_id: &str,
            _error_code: &str,
            _error_message: &str,
        ) -> Result<(), OrbitError> {
            Ok(())
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
        git(
            repo_dir.path(),
            &["config", "user.email", "orbit-tests@example.com"],
        );
        fs::write(repo_dir.path().join("README.md"), "orbit\n").expect("write readme");
        git(repo_dir.path(), &["add", "README.md"]);
        git(repo_dir.path(), &["commit", "-m", "init"]);
        git(
            repo_dir.path(),
            &["checkout", "-b", "orbit/T20260320-021158"],
        );
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
            agent: None,
            model: None,
            status: TaskStatus::Review,
            priority: TaskPriority::High,
            task_type: TaskType::Issue,
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
            }),
        )
        .expect("merge should succeed");

        assert_eq!(result["merged"], json!(true));

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
