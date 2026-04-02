use std::path::Path;

use orbit_types::OrbitError;
use serde_json::{Value, json};

use crate::context::RuntimeHost;

use super::git::git_success;
use super::input::{canonicalize_existing_dir, input_string_field};

pub(super) fn pull_batch_changes<H: RuntimeHost + ?Sized>(
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

    git_success(&workspace_path, &["pull", "--rebase"])?;
    Ok(json!({}))
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use serde_json::json;

    use orbit_types::{OrbitError, OrbitEvent};

    use super::pull_batch_changes;
    use crate::context::RuntimeHost;

    struct StubHost {
        repo_root: String,
    }

    impl RuntimeHost for StubHost {
        fn record_event(&self, _event: OrbitEvent) -> Result<(), OrbitError> {
            Ok(())
        }
        fn repo_root(&self) -> Result<String, OrbitError> {
            Ok(self.repo_root.clone())
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
            _input: serde_json::Value,
            _debug: bool,
        ) -> Result<crate::context::JobRunResult, OrbitError> {
            unimplemented!()
        }
        fn validate_activity_target_exists(
            &self,
            _target_type: orbit_types::JobTargetType,
            _target_id: &str,
        ) -> Result<orbit_types::Activity, OrbitError> {
            unimplemented!()
        }
        fn get_job(&self, _job_id: &str) -> Result<Option<orbit_types::Job>, OrbitError> {
            Ok(None)
        }
        fn run_tool_with_context_and_role(
            &self,
            _name: &str,
            _input: serde_json::Value,
            _role: orbit_types::Role,
            _tool_context: orbit_tools::ToolContext,
        ) -> Result<serde_json::Value, OrbitError> {
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
            Ok(())
        }
        fn scoring_enabled(&self) -> bool {
            false
        }
        fn scoreboard_dir(&self) -> &Path {
            Path::new(".")
        }
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .expect("run git");
        assert!(status.success(), "git {:?} failed", args);
    }

    #[test]
    fn pull_rebase_syncs_remote_commits() {
        // Set up a bare "remote" and a "worktree" clone.
        let remote_dir = tempfile::tempdir().expect("tempdir");
        let work_dir = tempfile::tempdir().expect("tempdir");

        // Init bare remote
        run_git(remote_dir.path(), &["init", "--bare"]);

        // Clone remote into work_dir
        let work_path = work_dir.path().join("repo");
        let status = Command::new("git")
            .args([
                "clone",
                &remote_dir.path().to_string_lossy(),
                &work_path.to_string_lossy(),
            ])
            .status()
            .expect("clone");
        assert!(status.success());

        run_git(&work_path, &["config", "user.name", "Test"]);
        run_git(&work_path, &["config", "user.email", "test@test.com"]);

        // Create initial commit and push
        std::fs::write(work_path.join("file.txt"), "v1\n").expect("write");
        run_git(&work_path, &["add", "file.txt"]);
        run_git(&work_path, &["commit", "-m", "initial"]);
        run_git(&work_path, &["push", "-u", "origin", "HEAD"]);

        // Simulate a CI commit pushed to the remote by cloning again and pushing
        let ci_dir = tempfile::tempdir().expect("tempdir");
        let ci_path = ci_dir.path().join("ci");
        let status = Command::new("git")
            .args([
                "clone",
                &remote_dir.path().to_string_lossy(),
                &ci_path.to_string_lossy(),
            ])
            .status()
            .expect("clone ci");
        assert!(status.success());

        run_git(&ci_path, &["config", "user.name", "CI"]);
        run_git(&ci_path, &["config", "user.email", "ci@test.com"]);
        std::fs::write(ci_path.join("formatted.txt"), "auto-format\n").expect("write");
        run_git(&ci_path, &["add", "formatted.txt"]);
        run_git(&ci_path, &["commit", "-m", "style: auto-format"]);
        run_git(&ci_path, &["push"]);

        // Now the work_dir is 1 commit behind. Pull should sync it.
        let host = StubHost { repo_root: work_path.to_string_lossy().to_string() };
        let input = json!({ "workspace_path": work_path.to_string_lossy() });
        let result = pull_batch_changes(&host, &input).expect("pull should succeed");
        assert_eq!(result, json!({}));

        // Verify the CI commit is now in the work_dir
        assert!(
            work_path.join("formatted.txt").exists(),
            "CI commit should be synced into worktree"
        );
    }

    #[test]
    fn pull_rebase_noop_when_up_to_date() {
        let remote_dir = tempfile::tempdir().expect("tempdir");
        let work_dir = tempfile::tempdir().expect("tempdir");

        run_git(remote_dir.path(), &["init", "--bare"]);

        let work_path = work_dir.path().join("repo");
        let status = Command::new("git")
            .args([
                "clone",
                &remote_dir.path().to_string_lossy(),
                &work_path.to_string_lossy(),
            ])
            .status()
            .expect("clone");
        assert!(status.success());

        run_git(&work_path, &["config", "user.name", "Test"]);
        run_git(&work_path, &["config", "user.email", "test@test.com"]);
        std::fs::write(work_path.join("file.txt"), "v1\n").expect("write");
        run_git(&work_path, &["add", "file.txt"]);
        run_git(&work_path, &["commit", "-m", "initial"]);
        run_git(&work_path, &["push", "-u", "origin", "HEAD"]);

        // Already up-to-date — should succeed as a no-op
        let host = StubHost { repo_root: work_path.to_string_lossy().to_string() };
        let input = json!({ "workspace_path": work_path.to_string_lossy() });
        let result = pull_batch_changes(&host, &input).expect("pull noop should succeed");
        assert_eq!(result, json!({}));
    }
}
