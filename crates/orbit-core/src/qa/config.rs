//! Resolved, validated view of the `[qa]` config section [ORB-10039,
//! reworked ORB-10146].
//!
//! Raw serde structs live in `config::raw`; this module turns them into a
//! fail-closed [`QaSweepConfig`] during `RuntimeConfig` load, so a malformed
//! `[qa]` section is a loud startup error everywhere — never a sweep that
//! silently validates nothing.
//!
//! qa-sweep v2 invokes a QA agent per workspace instead of running inline
//! shell checks. Legacy `[[qa.workspace.check]]` tables are rejected here with
//! a migration error rather than silently ignored.

use std::collections::BTreeSet;
use std::str::FromStr;
use std::time::Duration;

use orbit_common::types::{OrbitError, TaskPriority, TaskStatus};

use crate::config::{RawQaConfig, RawQaWorkspaceConfig};

/// Default ceiling priority for auto-filed QA tasks.
const DEFAULT_TASK_PRIORITY: TaskPriority = TaskPriority::Medium;
/// Default status for auto-filed QA tasks: `backlog`, so `ship-sweep` can
/// dispatch the fix unattended (design D4 — the loop closes without a human
/// courier). Set `qa.task_status = "proposed"` to require approval first.
const DEFAULT_TASK_STATUS: TaskStatus = TaskStatus::Backlog;
/// Default per-workspace agent-run wall-clock timeout, in minutes. Generous by
/// design: a QA agent builds, runs, and exercises new features hands-on.
const DEFAULT_AGENT_TIMEOUT_MINUTES: u64 = 120;
/// Default base URL of the loopback worker invoke daemon.
pub const DEFAULT_WORKER_BASE_URL: &str = "http://127.0.0.1:7879";

/// Host-level qa-sweep configuration (from the global `~/.orbit/config.toml`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QaSweepConfig {
    /// Ceiling priority for auto-filed QA tasks; a finding's severity-mapped
    /// priority is clamped to at most this.
    pub default_priority: TaskPriority,
    /// Status auto-filed QA tasks are created with (`backlog` or `proposed`).
    pub task_status: TaskStatus,
    /// Base URL of the loopback worker invoke daemon.
    pub worker_base_url: String,
    /// Direct-push workspaces to validate, in config order.
    pub workspaces: Vec<QaWorkspaceConfig>,
}

/// One workspace's QA-agent validation setup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QaWorkspaceConfig {
    /// Workspace name as registered in the global workspace registry.
    pub name: String,
    /// Branch the sweep expects the checkout to be on for validation; `None`
    /// falls back to the workspace's registered `base_branch`.
    pub branch: Option<String>,
    /// Named crew for the QA agent run; `None` falls back to the workspace's
    /// default crew resolution [ORB-10133].
    pub crew: Option<String>,
    /// Agent-run wall-clock timeout; the worker kills the run once it elapses.
    pub timeout: Duration,
    /// Cap on the number of commits listed in the QA prompt; `None` uses the
    /// built-in evidence cap.
    pub max_commits: Option<usize>,
}

impl Default for QaSweepConfig {
    fn default() -> Self {
        Self {
            default_priority: DEFAULT_TASK_PRIORITY,
            task_status: DEFAULT_TASK_STATUS,
            worker_base_url: DEFAULT_WORKER_BASE_URL.to_string(),
            workspaces: Vec::new(),
        }
    }
}

impl QaSweepConfig {
    /// Validate the raw `[qa]` section. `None` (section absent) resolves to an
    /// empty config — the sweep then reports "no workspaces configured".
    pub(crate) fn from_raw(raw: Option<&RawQaConfig>) -> Result<Self, OrbitError> {
        let Some(raw) = raw else {
            return Ok(Self::default());
        };

        let default_priority = match raw.default_priority.as_deref() {
            Some(value) => parse_priority("qa.default_priority", value)?,
            None => DEFAULT_TASK_PRIORITY,
        };
        let task_status = match raw.task_status.as_deref().map(str::trim) {
            None => DEFAULT_TASK_STATUS,
            Some("backlog") => TaskStatus::Backlog,
            Some("proposed") => TaskStatus::Proposed,
            Some(other) => {
                return Err(OrbitError::InvalidInput(format!(
                    "qa.task_status has invalid value '{other}'; expected one of: backlog, proposed"
                )));
            }
        };
        let worker_base_url = match raw.base_url.as_deref() {
            None => DEFAULT_WORKER_BASE_URL.to_string(),
            Some(value) => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    return Err(OrbitError::InvalidInput(
                        "qa.base_url must not be empty when set".to_string(),
                    ));
                }
                trimmed.trim_end_matches('/').to_string()
            }
        };

        let mut workspaces = Vec::new();
        let mut seen_workspaces = BTreeSet::new();
        for entry in raw.workspace.as_deref().unwrap_or_default() {
            let workspace = QaWorkspaceConfig::from_raw(entry)?;
            if !seen_workspaces.insert(workspace.name.clone()) {
                return Err(OrbitError::InvalidInput(format!(
                    "[[qa.workspace]] declares workspace '{}' more than once",
                    workspace.name
                )));
            }
            workspaces.push(workspace);
        }

        Ok(Self {
            default_priority,
            task_status,
            worker_base_url,
            workspaces,
        })
    }

    /// The configured entry for a workspace name, if any.
    pub fn workspace(&self, name: &str) -> Option<&QaWorkspaceConfig> {
        self.workspaces.iter().find(|ws| ws.name == name)
    }
}

impl QaWorkspaceConfig {
    fn from_raw(raw: &RawQaWorkspaceConfig) -> Result<Self, OrbitError> {
        let name = required_trimmed("[[qa.workspace]].name", raw.name.as_deref())?;

        // Fail-closed migration guard: qa-sweep v2 replaced inline shell checks
        // with a QA agent run, so a leftover `[[qa.workspace.check]]` table is a
        // stale config the operator must remove — never a silent no-op.
        if raw.check.is_some() {
            return Err(OrbitError::InvalidInput(format!(
                "[[qa.workspace]] '{name}' still declares [[qa.workspace.check]]; \
                 qa-sweep v2 (ORB-10146) removed inline shell checks — remove the \
                 check tables and configure a QA agent run via `crew` / `timeout_minutes` \
                 / `max_commits` instead"
            )));
        }

        let branch = match raw.branch.as_deref().map(str::trim) {
            Some("") => {
                return Err(OrbitError::InvalidInput(format!(
                    "[[qa.workspace]] '{name}': branch must not be empty when set"
                )));
            }
            other => other.map(ToOwned::to_owned),
        };
        let crew = match raw.crew.as_deref().map(str::trim) {
            Some("") => {
                return Err(OrbitError::InvalidInput(format!(
                    "[[qa.workspace]] '{name}': crew must not be empty when set"
                )));
            }
            other => other.map(ToOwned::to_owned),
        };
        let timeout_minutes = raw.timeout_minutes.unwrap_or(DEFAULT_AGENT_TIMEOUT_MINUTES);
        if timeout_minutes == 0 {
            return Err(OrbitError::InvalidInput(format!(
                "[[qa.workspace]] '{name}': timeout_minutes must be at least 1"
            )));
        }
        let max_commits = match raw.max_commits {
            Some(0) => {
                return Err(OrbitError::InvalidInput(format!(
                    "[[qa.workspace]] '{name}': max_commits must be at least 1 when set"
                )));
            }
            other => other,
        };

        Ok(Self {
            name,
            branch,
            crew,
            timeout: Duration::from_secs(timeout_minutes * 60),
            max_commits,
        })
    }
}

fn required_trimmed(label: &str, value: Option<&str>) -> Result<String, OrbitError> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| OrbitError::InvalidInput(format!("{label} must be set and non-empty")))
}

fn parse_priority(label: &str, value: &str) -> Result<TaskPriority, OrbitError> {
    TaskPriority::from_str(value.trim()).map_err(|_| {
        OrbitError::InvalidInput(format!(
            "{label} has invalid value '{value}'; expected one of: low, medium, high, critical"
        ))
    })
}
