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
//! # Registry contents
//! The builtin registry wires together the standard Orbit tool families:
//! filesystem mutation, git and GitHub helpers, Orbit task/job commands,
//! process spawning, network fetches, and time utilities. Each tool executes
//! inside a [`ToolContext`] that carries workspace boundaries, agent metadata,
//! process allowlists, and the narrow Orbit host surface used by Orbit builtins.
//!
//! # Dependency direction
//! `orbit-types` → `orbit-exec` → `orbit-tools` → orbit-engine, orbit-core

pub mod builtin;
pub mod external;
pub mod registry;

use std::path::PathBuf;
use std::sync::Arc;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrbitBuiltinAction {
    ActivityShow,
    ReviewThreadAdd,
    ReviewThreadList,
    ReviewThreadReply,
    ReviewThreadResolve,
    StateGet,
    StateSet,
    TaskAdd,
    TaskApprove,
    TaskDelete,
    TaskLint,
    TaskList,
    TaskLocks,
    TaskReject,
    TaskShow,
    TaskStart,
    TaskUpdate,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrbitTaskScope {
    pub orbit_root: Option<PathBuf>,
    pub task_id: Option<String>,
}

pub trait OrbitToolHost: Send + Sync {
    fn execute(
        &self,
        action: OrbitBuiltinAction,
        input: Value,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<Value, OrbitError>;

    fn task_scope(&self) -> OrbitTaskScope;
}

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
    /// Normalized agent name (e.g. `"claude"`). When set, GitHub tools auto-append
    /// an attribution footer to PR bodies and review comments.
    pub agent_name: Option<String>,
    /// Resolved model identifier (e.g. `"opus-4.6"`). Used alongside `agent_name`
    /// for the attribution footer.
    pub model_name: Option<String>,
    /// Program allowlist for `proc.spawn`. When non-empty, `proc.spawn` rejects
    /// any program not in this list. Empty means unrestricted.
    pub proc_allowed_programs: Vec<String>,
    /// Narrow Orbit application host used by Orbit builtins instead of respawning
    /// the Orbit CLI or carrying task-specific state in the generic tool context.
    pub orbit_host: Option<Arc<dyn OrbitToolHost>>,
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("cwd", &self.cwd)
            .field("allowed_tools", &self.allowed_tools)
            .field("workspace_root", &self.workspace_root)
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
    let value = input
        .get(key)
        .ok_or_else(|| OrbitError::InvalidInput(format!("missing `{key}`")))?;
    // Accept both strings and numbers (agents often pass numeric IDs without quotes).
    let raw = match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return Err(OrbitError::InvalidInput(format!("missing `{key}`"))),
    };
    let trimmed = raw.trim().to_string();
    if trimmed.is_empty() {
        return Err(OrbitError::InvalidInput(format!("missing `{key}`")));
    }
    Ok(trimmed)
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
