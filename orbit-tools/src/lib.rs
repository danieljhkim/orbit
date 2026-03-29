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
use std::sync::Arc;

use orbit_lock::FileLockChecker;
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

#[derive(Clone, Default)]
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
    /// Active Orbit task id for the current tool invocation when known.
    pub task_id: Option<String>,
    /// Shared file-lock checker used by fs tools for write/delete conflict prevention.
    pub file_lock_checker: Option<Arc<dyn FileLockChecker>>,
    /// Normalized agent name (e.g. `"claude"`). When set, GitHub tools auto-append
    /// an attribution footer to PR bodies and review comments.
    pub agent_name: Option<String>,
    /// Resolved model identifier (e.g. `"opus-4.6"`). Used alongside `agent_name`
    /// for the attribution footer.
    pub model_name: Option<String>,
    /// Program allowlist for `proc.spawn`. When non-empty, `proc.spawn` rejects
    /// any program not in this list. Empty means unrestricted.
    pub proc_allowed_programs: Vec<String>,
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("cwd", &self.cwd)
            .field("allowed_tools", &self.allowed_tools)
            .field("workspace_root", &self.workspace_root)
            .field("orbit_root", &self.orbit_root)
            .field("task_id", &self.task_id)
            .field(
                "has_file_lock_checker",
                &self
                    .file_lock_checker
                    .as_ref()
                    .map(|_| true)
                    .unwrap_or(false),
            )
            .field("agent_name", &self.agent_name)
            .field("model_name", &self.model_name)
            .field("proc_allowed_programs", &self.proc_allowed_programs)
            .finish()
    }
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
