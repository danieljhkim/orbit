use serde_json::Value;

use crate::OrbitError;

pub struct Workflow {
    pub alias: &'static str,
    pub job_id: &'static str,
}

pub const WORKFLOWS: &[Workflow] = &[
    Workflow {
        alias: "ship",
        job_id: "task_auto_pipeline",
    },
    Workflow {
        alias: "triage",
        job_id: "task_triage_pipeline",
    },
];

pub fn find_workflow(name: &str) -> Option<&'static Workflow> {
    WORKFLOWS.iter().find(|w| w.alias == name)
}

/// Canonical alias of the gated ship workflow (`task_auto_pipeline`).
pub const SHIP_WORKFLOW_ALIAS: &str = "ship";

/// Pipeline mode for the `ship` workflow: open a PR or apply locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShipMode {
    Pr,
    Local,
}

impl ShipMode {
    /// The value the `task_auto_pipeline` job expects in its `mode` input.
    pub fn as_input_value(self) -> &'static str {
        match self {
            ShipMode::Pr => "pr",
            ShipMode::Local => "local",
        }
    }

    /// Parse an external (CLI/HTTP) mode string.
    pub fn parse(value: &str) -> Result<Self, OrbitError> {
        match value {
            "pr" => Ok(ShipMode::Pr),
            "local" => Ok(ShipMode::Local),
            other => Err(OrbitError::InvalidInput(format!(
                "unknown ship mode '{other}' (expected 'pr' or 'local')"
            ))),
        }
    }
}

/// Resolve the effective ship mode for a workspace registry entry.
///
/// An explicit `ship_mode` on the entry wins; otherwise the mode defaults to
/// `local`. The default is deliberately NOT derived from `git_remote`: several
/// direct-commit workspaces (e.g. `worker`, `bridge`) still have GitHub remotes,
/// so a "github → pr" heuristic would wrongly attempt PRs for them. Only the
/// PR-gated workspaces (`orbit`, `sextant`) carry an explicit `ship_mode = "pr"`.
/// Defaulting to `local` means a sweep never accidentally attempts a PR.
///
/// An unparseable explicit `ship_mode` falls back to `local` rather than
/// erroring, so a malformed registry entry can never wedge a sweep.
pub fn resolved_ship_mode(workspace: &orbit_common::types::Workspace) -> ShipMode {
    match workspace.ship_mode.as_deref() {
        Some(explicit) => ShipMode::parse(explicit).unwrap_or(ShipMode::Local),
        None => ShipMode::Local,
    }
}

/// Build the `task_auto_pipeline` input document for a ship run.
///
/// An empty `task_ids` slice selects auto mode (the pipeline discovers
/// backlog tasks itself). Explicit ids are validated for duplicates and
/// emptiness so every submission surface rejects the same malformed input.
pub fn build_ship_input(
    mode: ShipMode,
    base_branch: &str,
    task_ids: &[String],
) -> Result<Value, OrbitError> {
    if base_branch.trim().is_empty() {
        return Err(OrbitError::InvalidInput(
            "ship base branch must not be empty".to_string(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for task_id in task_ids {
        if task_id.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task id in explicit task selection must not be empty".to_string(),
            ));
        }
        if !seen.insert(task_id.as_str()) {
            return Err(OrbitError::InvalidInput(format!(
                "duplicate task id '{task_id}' in explicit task selection"
            )));
        }
    }

    let mut map = serde_json::Map::new();
    map.insert(
        "mode".to_string(),
        Value::String(mode.as_input_value().to_string()),
    );
    map.insert(
        "base_branch".to_string(),
        Value::String(base_branch.to_string()),
    );
    if !task_ids.is_empty() {
        map.insert(
            "task_ids".to_string(),
            Value::Array(task_ids.iter().cloned().map(Value::String).collect()),
        );
    }
    Ok(Value::Object(map))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ship_workflow_routes_to_auto_pipeline_only() {
        let workflow = find_workflow("ship").expect("ship workflow");

        assert_eq!(workflow.job_id, "task_auto_pipeline");
        assert!(find_workflow("ship-auto").is_none());
        assert!(find_workflow("ship-local").is_none());
        assert!(find_workflow("review-pr").is_none());
    }

    #[test]
    fn triage_workflow_routes_to_triage_pipeline() {
        let workflow = find_workflow("triage").expect("triage workflow");

        assert_eq!(workflow.job_id, "task_triage_pipeline");
    }
}

#[cfg(test)]
mod ship_input_tests {
    use super::*;

    #[test]
    fn build_ship_input_auto_mode_omits_task_ids() {
        let input = build_ship_input(ShipMode::Pr, "main", &[]).expect("input builds");
        assert_eq!(input["mode"], "pr");
        assert_eq!(input["base_branch"], "main");
        assert!(input.get("task_ids").is_none());
    }

    #[test]
    fn build_ship_input_explicit_tasks_and_local_mode() {
        let task_ids = vec!["T1".to_string(), "T2".to_string()];
        let input = build_ship_input(ShipMode::Local, "agent-main", &task_ids).expect("builds");
        assert_eq!(input["mode"], "local");
        assert_eq!(input["base_branch"], "agent-main");
        assert_eq!(input["task_ids"], serde_json::json!(["T1", "T2"]));
    }

    #[test]
    fn build_ship_input_rejects_duplicates_blank_ids_and_empty_base() {
        let dup = vec!["T1".to_string(), "T1".to_string()];
        assert!(build_ship_input(ShipMode::Pr, "main", &dup).is_err());

        let blank = vec!["  ".to_string()];
        assert!(build_ship_input(ShipMode::Pr, "main", &blank).is_err());

        assert!(build_ship_input(ShipMode::Pr, "  ", &[]).is_err());
    }

    #[test]
    fn ship_mode_parses_external_strings() {
        assert_eq!(ShipMode::parse("pr").expect("pr"), ShipMode::Pr);
        assert_eq!(ShipMode::parse("local").expect("local"), ShipMode::Local);
        assert!(ShipMode::parse("yolo").is_err());
    }
}

#[cfg(test)]
mod ship_mode_resolution_tests {
    use super::*;
    use chrono::Utc;
    use orbit_common::types::{Workspace, WorkspaceStatus};

    fn workspace(git_remote: Option<&str>, ship_mode: Option<&str>) -> Workspace {
        Workspace {
            id: "ws_test".to_string(),
            name: "test".to_string(),
            owner_machine_id: None,
            git_remote: git_remote.map(str::to_string),
            ship_mode: ship_mode.map(str::to_string),
            base_branch: "agent-main".to_string(),
            status: WorkspaceStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn unset_ship_mode_defaults_to_local_regardless_of_remote() {
        // Direct-commit workspaces (worker, bridge) also have GitHub remotes,
        // so the default must NOT be derived from the remote — it is always local.
        assert_eq!(
            resolved_ship_mode(&workspace(Some("https://github.com/acme/worker.git"), None)),
            ShipMode::Local
        );
        assert_eq!(
            resolved_ship_mode(&workspace(Some("git@github.com:acme/bridge.git"), None)),
            ShipMode::Local
        );
        assert_eq!(
            resolved_ship_mode(&workspace(Some("/home/daniel/git/polaris.git"), None)),
            ShipMode::Local
        );
        assert_eq!(resolved_ship_mode(&workspace(None, None)), ShipMode::Local);
    }

    #[test]
    fn explicit_pr_wins_over_github_remote() {
        // The PR-gated repos (orbit, sextant) carry an explicit ship_mode = "pr".
        let ws = workspace(Some("https://github.com/acme/orbit.git"), Some("pr"));
        assert_eq!(resolved_ship_mode(&ws), ShipMode::Pr);
    }

    #[test]
    fn explicit_local_wins() {
        let ws = workspace(Some("https://github.com/acme/orbit.git"), Some("local"));
        assert_eq!(resolved_ship_mode(&ws), ShipMode::Local);
    }

    #[test]
    fn unparseable_explicit_mode_falls_back_to_local() {
        let ws = workspace(Some("https://github.com/acme/orbit.git"), Some("bogus"));
        assert_eq!(resolved_ship_mode(&ws), ShipMode::Local);
    }
}
