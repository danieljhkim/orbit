//! Builtin tool registry providing the standard Orbit toolset for agents and jobs.
//!
//! Implements and registers all built-in tools that agents can invoke during
//! activity execution: filesystem, git, GitHub, Orbit CLI, process, time, and
//! network tools. External (user-defined) tools are also supported via the registry.
//!
//! # Role
//! Depends on `orbit-exec` for process spawning and `orbit-types` for shared
//! types. Consumed by `orbit-engine` and `orbit-core`, which pass a configured
//! [`ToolRegistry`] into the execution context.
//!
//! # Key exports
//! - [`ToolRegistry`] — central registry; call `register_builtins()` to load all standard tools
//! - [`Tool`] trait — implement this to add a custom tool
//! - [`ToolContext`] — per-call context: cwd, allowed-tool allowlist, workspace root boundary
//! - [`require_str`] — helper to extract and validate string fields from tool input JSON
//! - [`check_exec_result`] — helper to turn a failed [`ExecutionResult`] into an `OrbitError`
//! - Timeout constants: [`TIMEOUT_FAST_MS`], [`TIMEOUT_DEFAULT_MS`], [`TIMEOUT_SLOW_MS`], [`TIMEOUT_LONG_MS`]
//!
//! # Dependency direction
//! `orbit-types` → `orbit-exec` → `orbit-tools` → orbit-engine, orbit-core

pub mod builtin;
pub mod external;
pub mod registry;

use std::path::PathBuf;

use serde_json::{Map, Value};

use orbit_types::{OrbitError, ToolSchema};

/// Fast operation timeout (1 s). Used for local command resolution (e.g. `which`).
pub const TIMEOUT_FAST_MS: u64 = 1_000;

/// Default network operation timeout (15 s). Used for most GitHub API calls
/// and Orbit CLI commands where a quick response is expected.
pub const TIMEOUT_DEFAULT_MS: u64 = 15_000;

/// Slow operation timeout (30 s). Used for git network operations and PR creation,
/// which may involve larger payloads or slower remotes.
pub const TIMEOUT_SLOW_MS: u64 = 30_000;

/// Long operation timeout (60 s). Used for `gh pr checkout`, which clones or
/// fetches a branch and may transfer significant data over the network.
pub const TIMEOUT_LONG_MS: u64 = 60_000;

pub use registry::ToolRegistry;

#[derive(Debug, Clone, Default)]
pub struct ToolContext {
    pub cwd: Option<String>,
    /// If non-empty, only tools in this list may be called. Empty means unrestricted.
    pub allowed_tools: Vec<String>,
    /// When set, fs tools enforce that all paths resolve inside this directory.
    /// Symlink escapes are blocked because paths are canonicalized before the check.
    /// If `None`, fs tools deny all access (fail-closed). The runtime pipeline
    /// auto-populates this from the data root's parent directory.
    pub workspace_root: Option<PathBuf>,
    /// The resolved `.orbit` data directory (e.g. `<repo>/.orbit`). Distinct from
    /// `workspace_root` (the repo root used for fs sandboxing) because the data
    /// directory can be redirected via `config.toml` or path overrides and is not
    /// always `<workspace_root>/.orbit`. When set, orbit tool calls inject
    /// `--root <path>` so the spawned orbit CLI resolves to the correct data root
    /// regardless of the agent's working directory (e.g. inside a git worktree).
    pub orbit_root: Option<PathBuf>,
    /// Normalized agent name (e.g. `"claude"`). When set, GitHub tools auto-append
    /// an attribution footer to PR bodies and review comments.
    pub agent_name: Option<String>,
    /// Resolved model identifier (e.g. `"opus-4.6"`). Used alongside `agent_name`
    /// for the attribution footer.
    pub model_name: Option<String>,
}

pub trait Tool: Send + Sync {
    fn schema(&self) -> ToolSchema;
    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError>;
}

/// Extract a non-empty string field from a tool input value.
///
/// Returns `Err(OrbitError::InvalidInput)` if the key is absent, not a string,
/// or contains only whitespace. The returned string is trimmed.
pub fn require_str(input: &Value, key: &str) -> Result<String, OrbitError> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| OrbitError::InvalidInput(format!("missing `{key}`")))
}

/// Assert that a process result succeeded, returning a descriptive error if not.
///
/// Use this instead of the repeated `if !result.success { return Err(...) }` pattern.
/// The `label` should be the command name (e.g. `"gh pr comment"`) and is included
/// in the error message for diagnostics.
pub fn check_exec_result(
    result: &orbit_types::ExecutionResult,
    label: &str,
) -> Result<(), OrbitError> {
    if result.success {
        Ok(())
    } else {
        Err(OrbitError::Execution(format!(
            "{label} failed: {}",
            result.stderr.trim()
        )))
    }
}

pub fn map_input_from_pairs(pairs: impl IntoIterator<Item = (String, String)>) -> Value {
    let mut map = Map::new();
    for (key, value) in pairs {
        map.insert(key, Value::String(value));
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use crate::{ToolContext, ToolRegistry};

    #[test]
    fn registry_contains_expected_builtins() {
        let mut registry = ToolRegistry::new();
        registry.register_builtins();
        let names = registry
            .schemas()
            .into_iter()
            .map(|s| s.name)
            .collect::<Vec<_>>();

        assert!(names.contains(&"fs.read".to_string()));
        assert!(names.contains(&"fs.write".to_string()));
        assert!(names.contains(&"git.stage_paths".to_string()));
        assert!(names.contains(&"git.commit".to_string()));
        assert!(names.contains(&"proc.spawn".to_string()));
        assert!(names.contains(&"time.now".to_string()));
        assert!(names.contains(&"github.auth.status".to_string()));
        assert!(names.contains(&"github.pr.create".to_string()));
    }

    #[test]
    fn fs_read_returns_file_contents() {
        let workspace = tempdir().expect("workspace dir");
        let path = workspace.path().join("note.txt");
        fs::write(&path, "hello").expect("write file");

        let ctx = ToolContext {
            workspace_root: Some(workspace.path().canonicalize().expect("canonicalize")),
            ..Default::default()
        };
        let mut registry = ToolRegistry::new();
        registry.register_builtins();

        let output = registry
            .execute(
                "fs.read",
                &ctx,
                json!({"path": path.to_string_lossy()}),
            )
            .expect("tool executes");

        assert_eq!(output["content"], "hello");
    }

    #[test]
    fn fs_read_denied_when_workspace_root_is_none() {
        let dir = tempdir().expect("temp dir");
        let path = dir.path().join("note.txt");
        fs::write(&path, "hello").expect("write file");

        let mut registry = ToolRegistry::new();
        registry.register_builtins();

        let err = registry
            .execute(
                "fs.read",
                &ToolContext::default(),
                json!({"path": path.to_string_lossy()}),
            )
            .expect_err("read with no workspace_root must be denied");
        assert!(
            err.to_string().contains("workspace_root is not set"),
            "expected fail-closed denial, got: {err}"
        );
    }

    #[test]
    fn fs_tools_inside_workspace_succeed() {
        let workspace = tempdir().expect("workspace dir");
        let file = workspace.path().join("data.txt");
        fs::write(&file, "contents").expect("write file");

        let ctx = ToolContext {
            workspace_root: Some(workspace.path().canonicalize().expect("canonicalize")),
            ..Default::default()
        };
        let mut registry = ToolRegistry::new();
        registry.register_builtins();

        let output = registry
            .execute("fs.read", &ctx, json!({"path": file.to_string_lossy()}))
            .expect("read inside workspace should succeed");
        assert_eq!(output["content"], "contents");

        registry
            .execute(
                "fs.list",
                &ctx,
                json!({"path": workspace.path().to_string_lossy()}),
            )
            .expect("list inside workspace should succeed");

        let write_target = workspace.path().join("out.txt");
        registry
            .execute(
                "fs.write",
                &ctx,
                json!({"path": write_target.to_string_lossy(), "content": "ok"}),
            )
            .expect("write inside workspace should succeed");

        registry
            .execute(
                "fs.delete",
                &ctx,
                json!({"path": write_target.to_string_lossy()}),
            )
            .expect("delete inside workspace should succeed");
    }

    #[test]
    fn fs_read_outside_workspace_is_denied() {
        let workspace = tempdir().expect("workspace dir");
        let outside = tempdir().expect("outside dir");
        let outside_file = outside.path().join("secret.txt");
        fs::write(&outside_file, "secret").expect("write outside file");

        let ctx = ToolContext {
            workspace_root: Some(workspace.path().canonicalize().expect("canonicalize")),
            ..Default::default()
        };
        let mut registry = ToolRegistry::new();
        registry.register_builtins();

        let err = registry
            .execute(
                "fs.read",
                &ctx,
                json!({"path": outside_file.to_string_lossy()}),
            )
            .expect_err("read outside workspace must be denied");
        assert!(
            err.to_string().contains("outside workspace"),
            "expected policy denied message, got: {err}"
        );
    }

    #[test]
    fn fs_write_outside_workspace_is_denied() {
        let workspace = tempdir().expect("workspace dir");
        let outside = tempdir().expect("outside dir");
        let outside_target = outside.path().join("injected.txt");

        let ctx = ToolContext {
            workspace_root: Some(workspace.path().canonicalize().expect("canonicalize")),
            ..Default::default()
        };
        let mut registry = ToolRegistry::new();
        registry.register_builtins();

        let err = registry
            .execute(
                "fs.write",
                &ctx,
                json!({"path": outside_target.to_string_lossy(), "content": "pwned"}),
            )
            .expect_err("write outside workspace must be denied");
        assert!(
            err.to_string().contains("outside workspace"),
            "expected policy denied message, got: {err}"
        );
    }
}
