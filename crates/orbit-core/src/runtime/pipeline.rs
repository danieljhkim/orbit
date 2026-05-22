use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use orbit_common::types::{OrbitEvent, Role, activity_job::tool_allowed};
use orbit_common::utility::redaction::{redact_sensitive_env_error, redact_sensitive_env_json};
use orbit_tools::ToolContext;
use serde_json::Value;

use crate::{NotFoundKind, OrbitError, OrbitRuntime};

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
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Tool, name.to_string()))?;

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
    if !task_workspace_matches(&canonical_repo_root, &canonical_cwd) {
        return Ok(None);
    }

    let tasks = runtime.stores().tasks().list()?;
    Ok(tasks.into_iter().next().map(|task| task.id))
}

fn resolve_workspace_root_from_context(
    runtime: &OrbitRuntime,
    task_id: Option<&str>,
    tool_context: &ToolContext,
) -> Result<Option<PathBuf>, OrbitError> {
    let canonical_repo_root = canonical_repo_root(runtime);
    if let Some(workspace_root) = active_git_checkout_root(&canonical_repo_root, tool_context) {
        return Ok(Some(workspace_root));
    }

    if let Some(task_id) = task_id
        && let Some(workspace_root) = resolve_task_workspace_root(runtime, task_id)
    {
        return Ok(Some(workspace_root));
    }
    Ok(Some(canonical_repo_root))
}

fn canonical_repo_root(runtime: &OrbitRuntime) -> PathBuf {
    runtime
        .context
        .paths()
        .repo_root
        .canonicalize()
        .unwrap_or_else(|_| runtime.context.paths().repo_root.clone())
}

fn resolve_task_workspace_root(runtime: &OrbitRuntime, task_id: &str) -> Option<PathBuf> {
    let repo_root = canonical_repo_root(runtime);
    runtime.get_task(task_id).ok()?;
    Some(repo_root)
}

fn active_git_checkout_root(
    canonical_repo_root: &Path,
    tool_context: &ToolContext,
) -> Option<PathBuf> {
    let cwd = tool_context.cwd.as_deref()?;
    let cwd = Path::new(cwd);
    let checkout_root = git_checkout_root(cwd)?;
    same_git_common_dir(&checkout_root, canonical_repo_root).then_some(checkout_root)
}

fn git_checkout_root(path: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let raw_path = stdout.lines().next()?.trim();
    if raw_path.is_empty() {
        return None;
    }
    let path = PathBuf::from(raw_path);
    Some(path.canonicalize().unwrap_or(path))
}

fn same_git_common_dir(left: &Path, right: &Path) -> bool {
    match (git_common_dir(left), git_common_dir(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn git_common_dir(path: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let raw_path = stdout.lines().next()?.trim();
    if raw_path.is_empty() {
        return None;
    }
    let path = PathBuf::from(raw_path);
    Some(path.canonicalize().unwrap_or(path))
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
