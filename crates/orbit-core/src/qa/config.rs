//! Resolved, validated view of the `[qa]` config section [ORB-10039].
//!
//! Raw serde structs live in `config::raw`; this module turns them into a
//! fail-closed [`QaSweepConfig`] during `RuntimeConfig` load, so a malformed
//! `[qa]` section is a loud startup error everywhere — never a sweep that
//! silently validates nothing.

use std::collections::BTreeSet;
use std::str::FromStr;
use std::time::Duration;

use orbit_common::types::{OrbitError, TaskPriority, TaskStatus};

use crate::config::{RawQaCheckConfig, RawQaConfig, RawQaWorkspaceConfig};

/// Default priority for auto-filed QA tasks.
const DEFAULT_TASK_PRIORITY: TaskPriority = TaskPriority::Medium;
/// Default status for auto-filed QA tasks: `backlog`, so `ship-sweep` can
/// dispatch the fix unattended (design D4 — the loop closes without a human
/// courier). Set `qa.task_status = "proposed"` to require approval first.
const DEFAULT_TASK_STATUS: TaskStatus = TaskStatus::Backlog;
/// Default per-check timeout.
const DEFAULT_CHECK_TIMEOUT_MINUTES: u64 = 30;

/// Host-level qa-sweep configuration (from the global `~/.orbit/config.toml`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QaSweepConfig {
    /// Priority for auto-filed QA tasks when a check does not override it.
    pub default_priority: TaskPriority,
    /// Status auto-filed QA tasks are created with (`backlog` or `proposed`).
    pub task_status: TaskStatus,
    /// Direct-push workspaces to validate, in config order.
    pub workspaces: Vec<QaWorkspaceConfig>,
}

/// One workspace's validation setup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QaWorkspaceConfig {
    /// Workspace name as registered in the global workspace registry.
    pub name: String,
    /// Branch the checkout must be on for the sweep to validate it; `None`
    /// falls back to the workspace's registered `base_branch`.
    pub branch: Option<String>,
    /// Checks run from the workspace root, in config order.
    pub checks: Vec<QaCheck>,
}

/// One configured check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QaCheck {
    /// Stable name; part of the failure fingerprint.
    pub name: String,
    /// Shell command run via `sh -c` from the workspace root.
    pub command: String,
    /// Muted checks are skipped without deleting their definition.
    pub mute: bool,
    /// Per-check priority override for auto-filed tasks.
    pub priority: Option<TaskPriority>,
    /// Kill the check and record a failure once this elapses.
    pub timeout: Duration,
}

impl Default for QaSweepConfig {
    fn default() -> Self {
        Self {
            default_priority: DEFAULT_TASK_PRIORITY,
            task_status: DEFAULT_TASK_STATUS,
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
        let branch = match raw.branch.as_deref().map(str::trim) {
            Some("") => {
                return Err(OrbitError::InvalidInput(format!(
                    "[[qa.workspace]] '{name}': branch must not be empty when set"
                )));
            }
            other => other.map(ToOwned::to_owned),
        };

        let raw_checks = raw.check.as_deref().unwrap_or_default();
        if raw_checks.is_empty() {
            return Err(OrbitError::InvalidInput(format!(
                "[[qa.workspace]] '{name}' must declare at least one [[qa.workspace.check]]"
            )));
        }
        let mut checks = Vec::new();
        let mut seen_checks = BTreeSet::new();
        for raw_check in raw_checks {
            let check = QaCheck::from_raw(&name, raw_check)?;
            if !seen_checks.insert(check.name.clone()) {
                return Err(OrbitError::InvalidInput(format!(
                    "[[qa.workspace]] '{name}' declares check '{}' more than once",
                    check.name
                )));
            }
            checks.push(check);
        }

        Ok(Self {
            name,
            branch,
            checks,
        })
    }
}

impl QaCheck {
    fn from_raw(workspace: &str, raw: &RawQaCheckConfig) -> Result<Self, OrbitError> {
        let context = format!("[[qa.workspace]] '{workspace}' check");
        let name = required_trimmed(&format!("{context} name"), raw.name.as_deref())?;
        let command = required_trimmed(
            &format!("{context} '{name}' command"),
            raw.command.as_deref(),
        )?;
        let priority = raw
            .priority
            .as_deref()
            .map(|value| parse_priority(&format!("{context} '{name}' priority"), value))
            .transpose()?;
        let timeout_minutes = raw.timeout_minutes.unwrap_or(DEFAULT_CHECK_TIMEOUT_MINUTES);
        if timeout_minutes == 0 {
            return Err(OrbitError::InvalidInput(format!(
                "{context} '{name}': timeout_minutes must be at least 1"
            )));
        }

        Ok(Self {
            name,
            command,
            mute: raw.mute.unwrap_or(false),
            priority,
            timeout: Duration::from_secs(timeout_minutes * 60),
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
