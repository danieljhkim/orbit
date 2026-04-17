use std::path::{Path, PathBuf};

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
        if let Some(stored) = self.stores().tools().get(name)?
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

    let tasks = runtime.stores().tasks().list()?;
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
