use std::path::Path;

use orbit_common::types::OrbitError;
use orbit_common::utility::fs::write_text_with_parent;

/// Shippable default activity assets, seeded under
/// `<orbit_root>/resources/activities/<name>.yaml` on `orbit init`. Keep this
/// list in sync with the workflow YAMLs under `crates/orbit-core/assets/jobs/`:
/// every `target: activity:<name>` reference in a shipped workflow must
/// resolve to an entry here. Reference/example activities (anything under
/// `assets/activities/examples/`) are deliberately excluded — they're
/// fixtures for `crates/orbit-engine/examples/v2_job_runtime_smoke.rs`, not
/// runtime defaults.
pub(crate) const DEFAULT_ACTIVITY_FILES: &[(&str, &str)] = &[
    (
        "agent_implement",
        include_str!("../../assets/activities/agent_implement.yaml"),
    ),
    (
        "agent_review",
        include_str!("../../assets/activities/agent_review.yaml"),
    ),
    (
        "apply_triage_dispositions",
        include_str!("../../assets/activities/apply_triage_dispositions.yaml"),
    ),
    (
        "dispatch_agent",
        include_str!("../../assets/activities/dispatch_agent.yaml"),
    ),
    (
        "epic_orchestrator",
        include_str!("../../assets/activities/epic_orchestrator.yaml"),
    ),
    (
        "gate_starvation_fail",
        include_str!("../../assets/activities/gate_starvation_fail.yaml"),
    ),
    (
        "git_merge",
        include_str!("../../assets/activities/git_merge.yaml"),
    ),
    (
        "git_commit",
        include_str!("../../assets/activities/git_commit.yaml"),
    ),
    (
        "git_push",
        include_str!("../../assets/activities/git_push.yaml"),
    ),
    (
        "git_rebase",
        include_str!("../../assets/activities/git_rebase.yaml"),
    ),
    (
        "invoke_and_wait",
        include_str!("../../assets/activities/invoke_and_wait.yaml"),
    ),
    (
        "independent_review_guard",
        include_str!("../../assets/activities/independent_review_guard.yaml"),
    ),
    (
        "pipeline_wait",
        include_str!("../../assets/activities/pipeline_wait.yaml"),
    ),
    (
        "pipeline_success_guard",
        include_str!("../../assets/activities/pipeline_success_guard.yaml"),
    ),
    (
        "list_backlog_tasks",
        include_str!("../../assets/activities/list_backlog_tasks.yaml"),
    ),
    (
        "list_triage_candidates",
        include_str!("../../assets/activities/list_triage_candidates.yaml"),
    ),
    (
        "load_epic",
        include_str!("../../assets/activities/load_epic.yaml"),
    ),
    (
        "pr_open",
        include_str!("../../assets/activities/pr_open.yaml"),
    ),
    (
        "pr_prepare",
        include_str!("../../assets/activities/pr_prepare.yaml"),
    ),
    (
        "pr_promote",
        include_str!("../../assets/activities/pr_promote.yaml"),
    ),
    (
        "reserve_locks",
        include_str!("../../assets/activities/reserve_locks.yaml"),
    ),
    (
        "release_locks",
        include_str!("../../assets/activities/release_locks.yaml"),
    ),
    (
        "resolve_workspace_ship_input",
        include_str!("../../assets/activities/resolve_workspace_ship_input.yaml"),
    ),
    (
        "run_auto_task_scheduler",
        include_str!("../../assets/activities/run_auto_task_scheduler.yaml"),
    ),
    (
        "run_planning_duel",
        include_str!("../../assets/activities/run_planning_duel.yaml"),
    ),
    ("sleep", include_str!("../../assets/activities/sleep.yaml")),
    (
        "step_failure_recovery",
        include_str!("../../assets/activities/step_failure_recovery.yaml"),
    ),
    (
        "summarize_epic",
        include_str!("../../assets/activities/summarize_epic.yaml"),
    ),
    (
        "triage_failed_runs",
        include_str!("../../assets/activities/triage_failed_runs.yaml"),
    ),
    (
        "update_task",
        include_str!("../../assets/activities/update_task.yaml"),
    ),
    (
        "validate_bundles",
        include_str!("../../assets/activities/validate_bundles.yaml"),
    ),
    (
        "worktree_setup",
        include_str!("../../assets/activities/worktree_setup.yaml"),
    ),
];

/// Seed every entry in [`DEFAULT_ACTIVITY_FILES`] as a YAML file under
/// `activities_dir`. Mirrors the skill / executor / policy seeding pattern:
/// the asset YAML is embedded in the binary via `include_str!` and copied
/// out on `orbit init` so the [`V2ActivityCatalog`] can discover it without
/// depending on a git checkout of this repo.
///
/// When `overwrite` is false, existing files are preserved — users who've
/// edited a previously-seeded activity won't lose their changes on re-init.
pub(crate) fn seed_default_activities(
    activities_dir: &Path,
    overwrite: bool,
) -> Result<usize, OrbitError> {
    let mut count = 0usize;
    for (name, content) in DEFAULT_ACTIVITY_FILES {
        let path = activities_dir.join(format!("{name}.yaml"));
        if !overwrite && path.exists() {
            continue;
        }
        write_text_with_parent(&path, content)?;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use orbit_common::types::activity_job::{AgentRole, OnDenial, tool_allowed};
    use orbit_common::types::{ActivityV2Spec, load_activity_asset};
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn seeded_deterministic_activities_match_actions() {
        let root = tempdir().expect("create tempdir");
        let activities_dir = root.path().join("resources/activities");
        seed_default_activities(&activities_dir, true).expect("seed default activities");

        for (name, action) in [
            ("run_planning_duel", "run_planning_duel"),
            ("git_commit", "git_commit"),
            ("git_rebase", "git_rebase"),
            ("pr_prepare", "pr_prepare"),
            ("pr_promote", "pr_promote"),
            ("release_locks", "release_locks"),
            ("pipeline_wait", "pipeline_wait"),
            ("independent_review_guard", "independent_review_guard"),
            ("list_triage_candidates", "list_triage_candidates"),
            ("apply_triage_dispositions", "apply_triage_dispositions"),
        ] {
            let yaml = std::fs::read_to_string(activities_dir.join(format!("{name}.yaml")))
                .unwrap_or_else(|error| panic!("read {name} activity: {error}"));
            let asset = load_activity_asset(&yaml)
                .unwrap_or_else(|error| panic!("parse {name} activity: {error}"));
            assert_eq!(asset.name, name);
            match asset.spec.spec {
                ActivityV2Spec::Deterministic(spec) => {
                    assert_eq!(spec.action, action);
                }
                other => panic!("expected deterministic {name} activity, got {other:?}"),
            }
        }
    }

    #[test]
    fn agent_implement_guidance_allows_bounded_scope_expansion() {
        let (_, yaml) = DEFAULT_ACTIVITY_FILES
            .iter()
            .find(|(name, _)| *name == "agent_implement")
            .expect("agent implement activity is seeded");
        let asset = load_activity_asset(yaml).expect("parse agent implement activity");
        match asset.spec.spec {
            ActivityV2Spec::AgentLoop(spec) => {
                let instruction = spec
                    .instruction
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                assert_eq!(spec.role, Some(AgentRole::Implementer));
                assert!(instruction.contains("not as a perfect inventory"));
                assert!(instruction.contains("make the smallest compatible change"));
                assert!(instruction.contains("Stop after commenting"));
            }
            other => panic!("expected agent_loop activity, got {other:?}"),
        }
    }

    #[test]
    fn agent_response_contract_matches_durable_handoff_shape() {
        for (name, required) in [
            ("agent_implement", false),
            ("agent_review", true),
            ("triage_failed_runs", true),
            ("epic_orchestrator", true),
        ] {
            let (_, yaml) = DEFAULT_ACTIVITY_FILES
                .iter()
                .find(|(candidate, _)| *candidate == name)
                .unwrap_or_else(|| panic!("{name} activity is seeded"));
            let asset = load_activity_asset(yaml)
                .unwrap_or_else(|error| panic!("parse {name} activity: {error}"));
            match asset.spec.spec {
                ActivityV2Spec::AgentLoop(spec) => assert_eq!(
                    spec.require_response_envelope, required,
                    "{name} response contract drifted"
                ),
                other => panic!("expected agent_loop {name} activity, got {other:?}"),
            }
        }
    }

    #[test]
    fn agent_review_is_read_only_and_requires_an_exact_head_verdict() {
        let (_, yaml) = DEFAULT_ACTIVITY_FILES
            .iter()
            .find(|(name, _)| *name == "agent_review")
            .expect("agent review activity is seeded");
        let asset = load_activity_asset(yaml).expect("parse agent review activity");
        assert_eq!(
            asset.spec.output_schema_json["required"],
            serde_json::json!(["verdict", "reviewed_head_sha"])
        );
        match asset.spec.spec {
            ActivityV2Spec::AgentLoop(spec) => {
                assert!(spec.require_response_envelope);
                assert_eq!(
                    spec.role, None,
                    "flat review crew must not need a role field"
                );
                assert!(!spec.tools.iter().any(|tool| tool == "fs.delete"));
                assert!(spec.instruction.contains("candidate_head_sha"));
                assert!(spec.instruction.contains("Do not edit"));
            }
            other => panic!("expected agent_loop agent_review, got {other:?}"),
        }
    }

    /// [ORB-10129] The triage agent's hard bounds are structural: its tool
    /// allowlist must exclude every write/dispatch surface (code edits,
    /// commits/pushes/merges, PR approval, pipeline invocation, task
    /// lifecycle writes), `proc_allowed_programs` must not include `git`,
    /// and `on_denial: terminate` must be set so a denied tool kills the
    /// run (termination semantics proven by
    /// `replay_denial_terminate_surfaces_structural_tool_denied` in
    /// orbit-engine).
    #[test]
    fn triage_agent_allowlist_makes_write_surfaces_structurally_impossible() {
        let (_, yaml) = DEFAULT_ACTIVITY_FILES
            .iter()
            .find(|(name, _)| *name == "triage_failed_runs")
            .expect("triage agent activity is seeded");
        let asset = load_activity_asset(yaml).expect("parse triage agent activity");
        match asset.spec.spec {
            ActivityV2Spec::AgentLoop(spec) => {
                assert_eq!(spec.role, Some(AgentRole::Reviewer));
                assert_eq!(spec.on_denial, OnDenial::Terminate);
                for denied in [
                    // code edits
                    "fs.write",
                    "fs.patch",
                    "fs.create",
                    "fs.move",
                    "fs.copy",
                    // pipeline / job dispatch
                    "orbit.pipeline.invoke",
                    "orbit.pipeline.wait",
                    // task lifecycle writes other than the activity's one
                    // evidence-gated blocked -> done reconciliation
                    "orbit.task.start",
                    "orbit.task.approve",
                    "orbit.task.reject",
                    "orbit.task.add",
                    "orbit.task.delete",
                    "orbit.task.locks.reserve",
                    "orbit.task.locks.release",
                ] {
                    assert!(
                        !tool_allowed(denied, &spec.tools),
                        "triage agent must not be able to call `{denied}`"
                    );
                }
                for allowed in [
                    "orbit.task.show",
                    "orbit.task.update",
                    "orbit.friction.add",
                    "fs.read",
                    "fs.delete",
                ] {
                    assert!(
                        tool_allowed(allowed, &spec.tools),
                        "triage agent should be able to call `{allowed}`"
                    );
                }
                // No `git` subprocess: commits/pushes/merges stay impossible
                // even through proc.spawn.
                assert_eq!(
                    spec.proc_allowed_programs.as_deref(),
                    Some(&["rg".to_string()][..])
                );
            }
            other => panic!("expected agent_loop activity, got {other:?}"),
        }
    }

    #[test]
    fn seeded_activities_include_step_failure_recovery() {
        let root = tempdir().expect("create tempdir");
        let activities_dir = root.path().join("resources/activities");
        seed_default_activities(&activities_dir, true).expect("seed default activities");

        let yaml = std::fs::read_to_string(activities_dir.join("step_failure_recovery.yaml"))
            .expect("read step failure recovery activity");
        let asset = load_activity_asset(&yaml).expect("parse step failure recovery activity");
        assert_eq!(asset.name, "step_failure_recovery");
        assert_eq!(
            asset.spec.input_schema_json["required"],
            serde_json::json!([
                "failed_step_id",
                "activity_name",
                "error_message",
                "attempt",
                "max_attempts"
            ])
        );
        assert_eq!(
            asset.spec.input_schema_json["additionalProperties"],
            serde_json::json!(false)
        );
        match asset.spec.spec {
            ActivityV2Spec::AgentLoop(spec) => {
                assert_eq!(spec.role, Some(AgentRole::Reviewer));
                assert!(!yaml.contains("\n  backend:"));
                assert!(!yaml.contains("\n  provider:"));
                assert!(
                    spec.instruction
                        .contains("You are Orbit's step-failure recovery agent.")
                );
            }
            other => panic!("expected agent_loop activity, got {other:?}"),
        }
    }
}
