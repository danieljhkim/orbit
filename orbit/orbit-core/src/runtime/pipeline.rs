use std::path::{Path, PathBuf};
use std::sync::Arc;

use orbit_lock::FileLockChecker;
use orbit_policy::PolicyContext;
use orbit_tools::ToolContext;
use orbit_types::{
    OrbitEvent, PolicyDecision, Role, redact_sensitive_env_error, redact_sensitive_env_json,
};
use serde_json::Value;

use crate::{OrbitError, OrbitRuntime};

impl OrbitRuntime {
    pub fn run_tool(&self, name: &str, input: Value) -> Result<Value, OrbitError> {
        self.run_tool_with_role(name, input, Role::Admin)
    }

    pub(crate) fn run_tool_with_role(
        &self,
        name: &str,
        input: Value,
        role: Role,
    ) -> Result<Value, OrbitError> {
        self.run_tool_with_context_and_role(name, input, role, ToolContext::default())
    }

    pub(crate) fn run_tool_with_context_and_role(
        &self,
        name: &str,
        input: Value,
        role: Role,
        mut tool_context: ToolContext,
    ) -> Result<Value, OrbitError> {
        if tool_context.cwd.is_none() {
            tool_context.cwd = std::env::current_dir()
                .ok()
                .map(|cwd| cwd.to_string_lossy().into_owned());
        }

        if tool_context.task_id.is_none() {
            tool_context.task_id = resolve_task_id_from_context(self, &tool_context)?;
        }

        // Ensure orbit tools always know the resolved data root so they can
        // inject --root into spawned orbit CLI calls (worktree-safe).
        if tool_context.orbit_root.is_none() {
            tool_context.orbit_root = Some(self.data_root_path().to_path_buf());
        }

        // Ensure fs tools always have a workspace boundary for sandboxing.
        if tool_context.workspace_root.is_none() {
            tool_context.workspace_root = resolve_workspace_root_from_context(self, &tool_context)?;
        }

        if tool_context.file_lock_checker.is_none() {
            let checker: Arc<dyn FileLockChecker> = self.context.file_lock_store().clone();
            tool_context.file_lock_checker = Some(checker);
        }

        self.check_tool_enabled(name)?;

        if !tool_context.allowed_tools.is_empty()
            && !tool_context.allowed_tools.iter().any(|t| t == name)
        {
            self.with_mutation(|| {
                Ok((
                    (),
                    OrbitEvent::PolicyDenied {
                        tool: name.to_string(),
                    },
                ))
            })?;
            return Err(OrbitError::PolicyDenied(format!(
                "tool '{name}' is not in the activity allowlist"
            )));
        }

        let decision = self.policy_engine().evaluate(&PolicyContext {
            entrypoint: "cli".to_string(),
            tool_name: Some(name.to_string()),
            role,
        });

        match decision {
            PolicyDecision::Deny { reason } => {
                self.with_mutation(|| {
                    Ok((
                        (),
                        OrbitEvent::PolicyDenied {
                            tool: name.to_string(),
                        },
                    ))
                })?;
                Err(OrbitError::PolicyDenied(reason))
            }
            PolicyDecision::Allow => {
                let output = self
                    .tool_registry()
                    .execute(name, &tool_context, input)
                    .map_err(redact_sensitive_env_error)?;
                let output = redact_sensitive_env_json(output);

                self.with_mutation(|| {
                    Ok((
                        (),
                        OrbitEvent::ToolExecuted {
                            name: name.to_string(),
                        },
                    ))
                })?;

                Ok(output)
            }
        }
    }

    pub fn run_tool_dry_run(&self, name: &str, input: &Value) -> Result<DryRunResult, OrbitError> {
        self.check_tool_enabled(name)?;

        let schema = self
            .tool_registry()
            .get_schema(name)
            .ok_or_else(|| OrbitError::ToolNotFound(name.to_string()))?;

        let decision = self.policy_engine().evaluate(&PolicyContext {
            entrypoint: "cli".to_string(),
            tool_name: Some(name.to_string()),
            role: Role::Admin,
        });

        let policy_allowed = matches!(decision, PolicyDecision::Allow);

        // Validate required parameters are present
        let mut missing_params = Vec::new();
        if let Some(obj) = input.as_object() {
            for param in &schema.parameters {
                if param.required && !obj.contains_key(&param.name) {
                    missing_params.push(param.name.clone());
                }
            }
        } else if !schema.parameters.is_empty() {
            for param in &schema.parameters {
                if param.required {
                    missing_params.push(param.name.clone());
                }
            }
        }

        Ok(DryRunResult {
            tool_name: name.to_string(),
            policy_allowed,
            missing_params,
        })
    }

    fn check_tool_enabled(&self, name: &str) -> Result<(), OrbitError> {
        if let Some(stored) = self.get_tool_record(name)?
            && !stored.enabled
        {
            return Err(OrbitError::Execution(format!(
                "tool '{name}' is disabled; enable it with: orbit tool enable {name}"
            )));
        }
        Ok(())
    }
}

fn resolve_task_id_from_context(
    runtime: &OrbitRuntime,
    tool_context: &ToolContext,
) -> Result<Option<String>, OrbitError> {
    let Some(cwd) = tool_context.cwd.as_deref() else {
        return Ok(None);
    };
    let canonical_cwd = match Path::new(cwd).canonicalize() {
        Ok(path) => path,
        Err(_) => PathBuf::from(cwd),
    };
    let canonical_repo_root = canonical_repo_root(runtime);

    let tasks = runtime.list_task_records()?;
    Ok(tasks
        .into_iter()
        .filter_map(|task| {
            let workspace =
                validated_task_workspace(&canonical_repo_root, task.workspace_path.as_deref()?)?;
            task_workspace_matches(&workspace, &canonical_cwd).then_some((task.id, workspace))
        })
        .max_by_key(|(_, workspace)| workspace.to_string_lossy().len())
        .map(|(task_id, _)| task_id))
}

fn resolve_workspace_root_from_context(
    runtime: &OrbitRuntime,
    tool_context: &ToolContext,
) -> Result<Option<PathBuf>, OrbitError> {
    if let Some(task_id) = tool_context.task_id.as_deref()
        && let Some(workspace_root) = resolve_task_workspace_root(runtime, task_id)
    {
        return Ok(Some(workspace_root));
    }
    Ok(Some(canonical_repo_root(runtime)))
}

fn canonical_repo_root(runtime: &OrbitRuntime) -> PathBuf {
    runtime
        .context
        .paths()
        .repo_root
        .canonicalize()
        .unwrap_or_else(|_| runtime.context.paths().repo_root.clone())
}

fn validated_task_workspace(repo_root: &Path, workspace_path: &str) -> Option<PathBuf> {
    let candidate = if Path::new(workspace_path).is_absolute() {
        PathBuf::from(workspace_path)
    } else {
        repo_root.join(workspace_path)
    };
    let canonical_workspace = candidate.canonicalize().ok()?;
    if !canonical_workspace.is_dir() {
        return None;
    }
    if canonical_workspace.starts_with(repo_root) {
        return Some(canonical_workspace);
    }

    let worktree_root = configured_worktree_root()?;
    canonical_workspace
        .starts_with(worktree_root)
        .then_some(canonical_workspace)
}

fn configured_worktree_root() -> Option<PathBuf> {
    let value = std::env::var("ORBIT_WORKTREE_ROOT").ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let path = PathBuf::from(trimmed);
    Some(path.canonicalize().unwrap_or(path))
}

fn resolve_task_workspace_root(runtime: &OrbitRuntime, task_id: &str) -> Option<PathBuf> {
    let repo_root = canonical_repo_root(runtime);
    let task = runtime.get_task(task_id).ok()?;
    let workspace_path = task.workspace_path.as_deref()?;
    validated_task_workspace(&repo_root, workspace_path)
}

fn task_workspace_matches(canonical_workspace: &Path, canonical_cwd: &Path) -> bool {
    canonical_cwd.starts_with(canonical_workspace)
}

#[derive(Debug, Clone)]
pub struct DryRunResult {
    pub tool_name: String,
    pub policy_allowed: bool,
    pub missing_params: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_repo_root, resolve_task_id_from_context, resolve_workspace_root_from_context,
    };
    use crate::OrbitRuntime;
    use orbit_store::TaskCreateParams as StoreTaskCreateParams;
    use orbit_tools::ToolContext;
    use orbit_types::{ActorIdentity, TaskPriority, TaskStatus, TaskType};
    use std::ffi::OsString;
    use std::fs;
    use std::sync::{Mutex, MutexGuard};

    static WORKTREE_ROOT_ENV_LOCK: Mutex<()> = Mutex::new(());
    const WORKTREE_ROOT_ENV: &str = "ORBIT_WORKTREE_ROOT";

    struct WorktreeRootEnvGuard {
        _lock: MutexGuard<'static, ()>,
        original: Option<OsString>,
    }

    impl Drop for WorktreeRootEnvGuard {
        fn drop(&mut self) {
            match self.original.take() {
                Some(value) => unsafe { std::env::set_var(WORKTREE_ROOT_ENV, value) },
                None => unsafe { std::env::remove_var(WORKTREE_ROOT_ENV) },
            }
        }
    }

    fn set_worktree_root_env(path: Option<&std::path::Path>) -> WorktreeRootEnvGuard {
        let lock = WORKTREE_ROOT_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let original = std::env::var_os(WORKTREE_ROOT_ENV);
        match path {
            Some(path) => unsafe { std::env::set_var(WORKTREE_ROOT_ENV, path) },
            None => unsafe { std::env::remove_var(WORKTREE_ROOT_ENV) },
        }
        WorktreeRootEnvGuard {
            _lock: lock,
            original,
        }
    }

    fn seed_task(runtime: &OrbitRuntime, title: &str, workspace_path: Option<String>) -> String {
        runtime
            .create_task_record(StoreTaskCreateParams {
                actor: "tester".to_string(),
                parent_id: None,
                title: title.to_string(),
                description: String::new(),
                acceptance_criteria: Vec::new(),
                plan: "## Plan\n- test".to_string(),
                execution_summary: String::new(),
                context_files: Vec::new(),
                workspace_path,
                repo_root: None,
                created_by: Some("tester".to_string()),
                actor_identity: ActorIdentity::System,
                assigned_to: Some("tester".to_string()),
                status: TaskStatus::InProgress,
                priority: TaskPriority::Medium,
                complexity: None,
                task_type: TaskType::Task,
                pr_number: None,
                proposed_by: None,
                source_task_id: None,
                comments: Vec::new(),
            })
            .expect("task")
            .id
    }

    #[test]
    fn invalid_workspace_root_falls_back_to_repo_root() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let outside = tempfile::tempdir().expect("tempdir");
        let task_id = seed_task(
            &runtime,
            "escaped workspace",
            Some(outside.path().to_string_lossy().into_owned()),
        );

        let resolved = resolve_workspace_root_from_context(
            &runtime,
            &ToolContext {
                task_id: Some(task_id),
                ..Default::default()
            },
        )
        .expect("workspace root")
        .expect("workspace path");

        assert_eq!(resolved, canonical_repo_root(&runtime));
    }

    #[test]
    fn task_resolution_prefers_most_specific_matching_workspace() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let repo_root = canonical_repo_root(&runtime);
        let nested = repo_root.join("nested");
        let deeper = nested.join("deeper");
        fs::create_dir_all(&deeper).expect("workspace dirs");

        let broad_id = seed_task(
            &runtime,
            "broad",
            Some(nested.to_string_lossy().into_owned()),
        );
        let specific_id = seed_task(
            &runtime,
            "specific",
            Some(deeper.to_string_lossy().into_owned()),
        );

        let resolved = resolve_task_id_from_context(
            &runtime,
            &ToolContext {
                cwd: Some(deeper.to_string_lossy().into_owned()),
                ..Default::default()
            },
        )
        .expect("task id");

        assert_eq!(resolved, Some(specific_id));
        assert_ne!(resolved, Some(broad_id));
    }

    #[test]
    fn external_worktree_root_is_used_for_task_resolution_and_workspace_root() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let repo_root = canonical_repo_root(&runtime);
        let external_root = tempfile::tempdir().expect("tempdir");
        let external_workspace = external_root
            .path()
            .join(
                repo_root
                    .file_name()
                    .expect("repo name")
                    .to_string_lossy()
                    .as_ref(),
            )
            .join("parallel-batch");
        let nested_cwd = external_workspace.join("orbit").join("orbit-core");
        fs::create_dir_all(&nested_cwd).expect("external workspace");

        let _env = set_worktree_root_env(Some(external_root.path()));
        let task_id = seed_task(
            &runtime,
            "external worktree",
            Some(external_workspace.to_string_lossy().into_owned()),
        );

        let resolved_task = resolve_task_id_from_context(
            &runtime,
            &ToolContext {
                cwd: Some(nested_cwd.to_string_lossy().into_owned()),
                ..Default::default()
            },
        )
        .expect("task id");
        let resolved_workspace = resolve_workspace_root_from_context(
            &runtime,
            &ToolContext {
                task_id: Some(task_id.clone()),
                ..Default::default()
            },
        )
        .expect("workspace root")
        .expect("workspace path");

        let canonical_external_workspace = external_workspace.canonicalize().expect("canonicalize");
        assert_eq!(resolved_task, Some(task_id));
        assert_eq!(resolved_workspace, canonical_external_workspace);
    }

    #[test]
    fn repo_worktree_paths_still_resolve_without_external_root_env() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let repo_root = canonical_repo_root(&runtime);
        let in_repo_workspace = repo_root.join(".orbit").join("worktrees").join("task-123");
        let nested_cwd = in_repo_workspace.join("orbit").join("orbit-core");
        fs::create_dir_all(&nested_cwd).expect("workspace dirs");

        let _env = set_worktree_root_env(None);
        let task_id = seed_task(
            &runtime,
            "repo worktree",
            Some(in_repo_workspace.to_string_lossy().into_owned()),
        );

        let resolved_task = resolve_task_id_from_context(
            &runtime,
            &ToolContext {
                cwd: Some(nested_cwd.to_string_lossy().into_owned()),
                ..Default::default()
            },
        )
        .expect("task id");

        assert_eq!(resolved_task, Some(task_id));
    }
}
