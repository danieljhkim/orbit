use serde_json::Value;

use crate::OrbitError;

pub struct Workflow {
    pub alias: &'static str,
    pub job_id: &'static str,
    pub description: &'static str,
    pub supports_tasks: bool,
    pub supports_parallelism: bool,
    pub supports_base: bool,
    pub supports_pr_number: bool,
    pub requires_pr_number: bool,
    /// Upper bound on explicit task-selection cardinality. `None` means unbounded (the
    /// historical default). Set to `Some(1)` for single-task workflows like
    /// `duel-plan` that must reject multi-task input with a loud, workflow-
    /// specific error rather than silently taking the first entry.
    pub max_tasks: Option<u32>,
}

pub const WORKFLOWS: &[Workflow] = &[
    Workflow {
        alias: "ship",
        job_id: "task_auto_pipeline",
        description: "Gate and ship backlog or explicitly selected tasks",
        supports_tasks: true,
        supports_parallelism: false,
        supports_base: true,
        supports_pr_number: false,
        requires_pr_number: false,
        max_tasks: None,
    },
    Workflow {
        alias: "review-pr",
        job_id: "job_batch_review_cycle",
        description: "Review, gate, fix-loop, and merge a batch PR by PR number",
        supports_tasks: false,
        supports_parallelism: false,
        supports_base: true,
        supports_pr_number: true,
        requires_pr_number: true,
        max_tasks: None,
    },
    Workflow {
        alias: "triage",
        job_id: "task_triage_pipeline",
        description: "Triage tasks blocked by failed runs; re-backlog environmental failures",
        supports_tasks: true,
        supports_parallelism: false,
        supports_base: false,
        supports_pr_number: false,
        requires_pr_number: false,
        max_tasks: None,
    },
    Workflow {
        alias: "duel-plan",
        job_id: "job_duel_plan_pipeline",
        description: "Single-task planning duel: two planners and one arbiter, scored",
        supports_tasks: true,
        supports_parallelism: false,
        supports_base: true,
        supports_pr_number: false,
        requires_pr_number: false,
        max_tasks: Some(1),
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
/// Review controls are omitted from the resulting document when disabled to
/// preserve the historical ship input exactly. Enabling review requires a
/// non-blank explicit crew so review never inherits implementation routing.
pub fn build_ship_input(
    mode: ShipMode,
    base_branch: &str,
    task_ids: &[String],
    review: bool,
    review_crew: Option<&str>,
) -> Result<Value, OrbitError> {
    if base_branch.trim().is_empty() {
        return Err(OrbitError::InvalidInput(
            "ship base branch must not be empty".to_string(),
        ));
    }
    if review && mode != ShipMode::Pr {
        return Err(OrbitError::InvalidInput(
            "ship review is supported only for PR mode".to_string(),
        ));
    }
    if review && task_ids.is_empty() {
        return Err(OrbitError::InvalidInput(
            "ship review requires an explicit task id selection".to_string(),
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
    if review {
        let review_crew = review_crew
            .map(str::trim)
            .filter(|crew| !crew.is_empty())
            .ok_or_else(|| {
                OrbitError::InvalidInput(
                    "ship review requires a non-blank explicit review crew".to_string(),
                )
            })?;
        map.insert("review".to_string(), Value::Bool(true));
        map.insert(
            "review_crew".to_string(),
            Value::String(review_crew.to_string()),
        );
    }
    Ok(Value::Object(map))
}

pub struct WorkflowInput {
    pub tasks: Option<String>,
    pub parallelism: Option<u32>,
    pub base: Option<String>,
    pub pr_number: Option<String>,
}

pub fn validate_workflow_flags(
    workflow: &Workflow,
    input: &WorkflowInput,
) -> Result<(), OrbitError> {
    if !workflow.supports_tasks && input.tasks.is_some() {
        return Err(OrbitError::InvalidInput(format!(
            "explicit task selection is not supported by workflow '{}'",
            workflow.alias
        )));
    }
    if !workflow.supports_parallelism && input.parallelism.is_some() {
        return Err(OrbitError::InvalidInput(format!(
            "--parallelism is not supported by workflow '{}'",
            workflow.alias
        )));
    }
    if !workflow.supports_base && input.base.is_some() {
        return Err(OrbitError::InvalidInput(format!(
            "--base is not supported by workflow '{}'",
            workflow.alias
        )));
    }
    if !workflow.supports_pr_number && input.pr_number.is_some() {
        return Err(OrbitError::InvalidInput(format!(
            "--pr-number is not supported by workflow '{}'",
            workflow.alias
        )));
    }
    if workflow.requires_pr_number && input.pr_number.is_none() {
        return Err(OrbitError::InvalidInput(format!(
            "--pr-number is required for workflow '{}'",
            workflow.alias
        )));
    }
    Ok(())
}

pub fn build_workflow_input(input: &WorkflowInput) -> Result<Value, OrbitError> {
    build_workflow_input_for(None, input)
}

/// Variant of [`build_workflow_input`] that also enforces any workflow-
/// specific cardinality constraints such as `Workflow::max_tasks`. Callers
/// that already know the resolved workflow should use this; the legacy
/// `build_workflow_input` is retained for call sites that do not.
pub fn build_workflow_input_for(
    workflow: Option<&Workflow>,
    input: &WorkflowInput,
) -> Result<Value, OrbitError> {
    let mut map = serde_json::Map::new();

    if let Some(tasks) = &input.tasks {
        let task_ids: Vec<Value> = tasks
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| Value::String(s.to_string()))
            .collect();
        if task_ids.is_empty() {
            return Err(OrbitError::InvalidInput(
                "task id selection must not be empty".to_string(),
            ));
        }
        if let Some(workflow) = workflow
            && let Some(max) = workflow.max_tasks
            && task_ids.len() as u32 > max
        {
            if max == 1 {
                return Err(OrbitError::InvalidInput(format!(
                    "workflow '{}' accepts exactly one task id — got {}",
                    workflow.alias,
                    task_ids.len()
                )));
            }
            return Err(OrbitError::InvalidInput(format!(
                "workflow '{}' accepts at most {} task ids — got {}",
                workflow.alias,
                max,
                task_ids.len()
            )));
        }
        if let Some(workflow) = workflow
            && workflow.max_tasks == Some(1)
            && task_ids.len() == 1
        {
            map.insert("task_id".to_string(), task_ids[0].clone());
        }
        map.insert("task_ids".to_string(), Value::Array(task_ids));
    }

    if let Some(parallelism) = input.parallelism {
        if parallelism == 0 {
            return Err(OrbitError::InvalidInput(
                "--parallelism must be greater than 0".to_string(),
            ));
        }
        map.insert("parallelism".to_string(), Value::Number(parallelism.into()));
    }

    if let Some(base) = &input.base {
        if base.is_empty() {
            return Err(OrbitError::InvalidInput(
                "--base must not be empty".to_string(),
            ));
        }
        map.insert("base".to_string(), Value::String(base.clone()));
    }

    if let Some(pr_number) = &input.pr_number {
        if pr_number.is_empty() {
            return Err(OrbitError::InvalidInput(
                "--pr-number must not be empty".to_string(),
            ));
        }
        map.insert("pr_number".to_string(), Value::String(pr_number.clone()));
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
        assert!(workflow.supports_tasks);
        assert!(workflow.supports_base);
        assert!(!workflow.supports_parallelism);
        assert!(find_workflow("ship-auto").is_none());
        assert!(find_workflow("ship-local").is_none());
    }

    #[test]
    fn triage_workflow_routes_to_triage_pipeline() {
        let workflow = find_workflow("triage").expect("triage workflow");

        assert_eq!(workflow.job_id, "task_triage_pipeline");
        assert!(workflow.supports_tasks);
        assert!(!workflow.supports_base);
        assert!(!workflow.supports_parallelism);
        assert!(!workflow.supports_pr_number);
    }
}

#[cfg(test)]
mod ship_input_tests {
    use super::*;

    #[test]
    fn build_ship_input_auto_mode_omits_task_ids() {
        let input = build_ship_input(ShipMode::Pr, "main", &[], false, None).expect("input builds");
        assert_eq!(input["mode"], "pr");
        assert_eq!(input["base_branch"], "main");
        assert!(input.get("task_ids").is_none());
        assert!(input.get("review").is_none());
        assert!(input.get("review_crew").is_none());
    }

    #[test]
    fn build_ship_input_explicit_tasks_and_local_mode() {
        let task_ids = vec!["T1".to_string(), "T2".to_string()];
        let input = build_ship_input(ShipMode::Local, "agent-main", &task_ids, false, None)
            .expect("builds");
        assert_eq!(input["mode"], "local");
        assert_eq!(input["base_branch"], "agent-main");
        assert_eq!(input["task_ids"], serde_json::json!(["T1", "T2"]));
    }

    #[test]
    fn build_ship_input_rejects_duplicates_blank_ids_and_empty_base() {
        let dup = vec!["T1".to_string(), "T1".to_string()];
        assert!(build_ship_input(ShipMode::Pr, "main", &dup, false, None).is_err());

        let blank = vec!["  ".to_string()];
        assert!(build_ship_input(ShipMode::Pr, "main", &blank, false, None).is_err());

        assert!(build_ship_input(ShipMode::Pr, "  ", &[], false, None).is_err());
    }

    #[test]
    fn build_ship_input_includes_explicit_review_controls() {
        let input = build_ship_input(
            ShipMode::Pr,
            "main",
            &["ORB-10000".to_string()],
            true,
            Some("opus-review"),
        )
        .expect("review input builds");

        assert_eq!(input["review"], true);
        assert_eq!(input["review_crew"], "opus-review");
        assert_eq!(input["task_ids"], serde_json::json!(["ORB-10000"]));
    }

    #[test]
    fn build_ship_input_rejects_enabled_review_without_non_blank_crew() {
        for review_crew in [None, Some(""), Some("   ")] {
            let error = build_ship_input(
                ShipMode::Pr,
                "main",
                &["ORB-10000".to_string()],
                true,
                review_crew,
            )
            .expect_err("enabled review requires a crew");
            assert!(
                error.to_string().contains("non-blank explicit review crew"),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    fn build_ship_input_rejects_review_without_explicit_pr_tasks() {
        let auto_error = build_ship_input(ShipMode::Pr, "main", &[], true, Some("opus"))
            .expect_err("review cannot auto-discover tasks");
        assert!(auto_error.to_string().contains("explicit task id"));

        let local_error = build_ship_input(
            ShipMode::Local,
            "main",
            &["ORB-10000".to_string()],
            true,
            Some("opus"),
        )
        .expect_err("review is PR-only");
        assert!(local_error.to_string().contains("only for PR mode"));
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
