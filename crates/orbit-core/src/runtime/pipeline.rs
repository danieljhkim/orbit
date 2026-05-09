use std::path::{Path, PathBuf};
use std::sync::Arc;

use orbit_common::types::{OrbitEvent, Role, activity_job::tool_allowed};
use orbit_common::utility::redaction::{redact_sensitive_env_error, redact_sensitive_env_json};
use orbit_tools::ToolContext;
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
        _role: Role,
        mut tool_context: ToolContext,
    ) -> Result<Value, OrbitError> {
        if tool_context.cwd.is_none() {
            tool_context.cwd = std::env::current_dir()
                .ok()
                .map(|cwd| cwd.to_string_lossy().into_owned());
        }

        let resolved_task_id = match tool_context.orbit_host.as_ref() {
            Some(host) => host.task_scope().task_id,
            None => resolve_task_id_from_context(self, &tool_context)?,
        };

        if tool_context.orbit_host.is_none() {
            tool_context.orbit_host =
                Some(super::build_orbit_tool_host(self, resolved_task_id.clone()));
        }

        // Ensure fs tools always have a workspace boundary for sandboxing.
        if tool_context.workspace_root.is_none() {
            tool_context.workspace_root = resolve_workspace_root_from_context(
                self,
                resolved_task_id.as_deref(),
                &tool_context,
            )?;
        }
        if tool_context.policy_engine.is_none() {
            tool_context.policy_engine = Some(Arc::new(self.policy_engine().clone()));
        }
        if tool_context.fs_profile.is_none() {
            tool_context.fs_profile = read_activity_fs_profile_from_env();
        }

        self.check_tool_enabled(name)?;

        if !tool_context.allowed_tools.is_empty()
            && !tool_allowed(name, &tool_context.allowed_tools)
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

        let output = match self
            .tool_registry()
            .execute(name, &tool_context, input)
            .map_err(redact_sensitive_env_error)
        {
            Ok(output) => output,
            Err(OrbitError::PolicyDenied(reason)) => {
                self.with_mutation(|| {
                    Ok((
                        (),
                        OrbitEvent::PolicyDenied {
                            tool: name.to_string(),
                        },
                    ))
                })?;
                return Err(OrbitError::PolicyDenied(reason));
            }
            Err(error) => return Err(error),
        };
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

    pub fn run_tool_dry_run(&self, name: &str, input: &Value) -> Result<DryRunResult, OrbitError> {
        self.check_tool_enabled(name)?;

        let schema = self
            .tool_registry()
            .get_schema(name)
            .ok_or_else(|| OrbitError::ToolNotFound(name.to_string()))?;

        let mut tool_context = ToolContext {
            cwd: std::env::current_dir()
                .ok()
                .map(|cwd| cwd.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let task_id = resolve_task_id_from_context(self, &tool_context)?;
        tool_context.workspace_root =
            resolve_workspace_root_from_context(self, task_id.as_deref(), &tool_context)?;

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
            policy_allowed: true,
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
    task_id: Option<&str>,
    _tool_context: &ToolContext,
) -> Result<Option<PathBuf>, OrbitError> {
    if let Some(task_id) = task_id
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

fn read_activity_fs_profile_from_env() -> Option<String> {
    let value = std::env::var("ORBIT_ACTIVITY_FS_PROFILE").ok()?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed.to_string())
}

#[derive(Debug, Clone)]
pub struct DryRunResult {
    pub tool_name: String,
    pub policy_allowed: bool,
    pub missing_params: Vec<String>,
}

#[cfg(test)]
mod tests {
    use orbit_common::types::{TaskPriority, TaskStatus, TaskType};
    use orbit_store::TaskCreateParams;
    use orbit_tools::ToolContext;
    use serde_json::json;

    use super::*;

    #[test]
    fn run_tool_context_allowlist_honors_task_wildcard() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let task = runtime
            .stores()
            .tasks()
            .create(TaskCreateParams {
                actor: "test".to_string(),
                parent_id: None,
                title: "Wildcard task".to_string(),
                description: "Exercise wildcard runtime allowlist".to_string(),
                acceptance_criteria: Vec::new(),
                dependencies: Vec::new(),
                plan: String::new(),
                execution_summary: String::new(),
                context_files: Vec::new(),
                workspace_path: Some(runtime.paths().repo_root.to_string_lossy().into_owned()),
                repo_root: None,
                created_by: Some("test".to_string()),
                planned_by: None,
                implemented_by: None,
                agent: None,
                model: None,
                status: TaskStatus::Backlog,
                priority: TaskPriority::Medium,
                complexity: None,
                task_type: TaskType::Task,
                external_refs: Vec::new(),
                source_task_id: None,
                comments: Vec::new(),
            })
            .expect("create task");

        let output = runtime
            .run_tool_with_context_and_role(
                "orbit.task.show",
                json!({ "id": task.id.clone() }),
                Role::Admin,
                ToolContext {
                    allowed_tools: vec!["orbit.task.*".to_string()],
                    orbit_host: Some(crate::runtime::build_orbit_tool_host(
                        &runtime,
                        Some(task.id.clone()),
                    )),
                    ..Default::default()
                },
            )
            .expect("wildcard activity context should permit orbit.task.show");

        assert_eq!(output["id"], task.id);
    }
}
