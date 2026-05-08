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
        "invoke_and_wait",
        include_str!("../../assets/activities/invoke_and_wait.yaml"),
    ),
    (
        "pipeline_wait",
        include_str!("../../assets/activities/pipeline_wait.yaml"),
    ),
    (
        "list_backlog_tasks",
        include_str!("../../assets/activities/list_backlog_tasks.yaml"),
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
        "reserve_locks",
        include_str!("../../assets/activities/reserve_locks.yaml"),
    ),
    (
        "release_locks",
        include_str!("../../assets/activities/release_locks.yaml"),
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
    use orbit_common::types::activity_job::{Backend, Provider};
    use orbit_common::types::{ActivityV2Spec, load_activity_asset};
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn seeded_activities_include_planning_duel_runner() {
        let root = tempdir().expect("create tempdir");
        let activities_dir = root.path().join("resources/activities");
        seed_default_activities(&activities_dir, true).expect("seed default activities");

        let yaml = std::fs::read_to_string(activities_dir.join("run_planning_duel.yaml"))
            .expect("read planning duel activity");
        let asset = load_activity_asset(&yaml).expect("parse planning duel activity");
        assert_eq!(asset.name, "run_planning_duel");
        match asset.spec.spec {
            ActivityV2Spec::Deterministic(spec) => {
                assert_eq!(spec.action, "run_planning_duel");
            }
            other => panic!("expected deterministic activity, got {other:?}"),
        }
    }

    #[test]
    fn seeded_activities_include_git_commit() {
        let root = tempdir().expect("create tempdir");
        let activities_dir = root.path().join("resources/activities");
        seed_default_activities(&activities_dir, true).expect("seed default activities");

        let yaml = std::fs::read_to_string(activities_dir.join("git_commit.yaml"))
            .expect("read git commit activity");
        let asset = load_activity_asset(&yaml).expect("parse git commit activity");
        assert_eq!(asset.name, "git_commit");
        match asset.spec.spec {
            ActivityV2Spec::Deterministic(spec) => {
                assert_eq!(spec.action, "git_commit");
            }
            other => panic!("expected deterministic activity, got {other:?}"),
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
                assert_eq!(spec.backend, Backend::Cli);
                assert_eq!(spec.provider, Provider::Codex);
                assert!(
                    spec.instruction
                        .contains("You are Orbit's step-failure recovery agent.")
                );
            }
            other => panic!("expected agent_loop activity, got {other:?}"),
        }
    }

    #[test]
    fn seeded_activities_include_release_locks() {
        let root = tempdir().expect("create tempdir");
        let activities_dir = root.path().join("resources/activities");
        seed_default_activities(&activities_dir, true).expect("seed default activities");

        let yaml = std::fs::read_to_string(activities_dir.join("release_locks.yaml"))
            .expect("read release locks activity");
        let asset = load_activity_asset(&yaml).expect("parse release locks activity");
        assert_eq!(asset.name, "release_locks");
        match asset.spec.spec {
            ActivityV2Spec::Deterministic(spec) => {
                assert_eq!(spec.action, "release_locks");
            }
            other => panic!("expected deterministic activity, got {other:?}"),
        }
    }

    #[test]
    fn seeded_activities_include_pipeline_wait() {
        let root = tempdir().expect("create tempdir");
        let activities_dir = root.path().join("resources/activities");
        seed_default_activities(&activities_dir, true).expect("seed default activities");

        let yaml = std::fs::read_to_string(activities_dir.join("pipeline_wait.yaml"))
            .expect("read pipeline wait activity");
        let asset = load_activity_asset(&yaml).expect("parse pipeline wait activity");
        assert_eq!(asset.name, "pipeline_wait");
        match asset.spec.spec {
            ActivityV2Spec::Deterministic(spec) => {
                assert_eq!(spec.action, "pipeline_wait");
            }
            other => panic!("expected deterministic activity, got {other:?}"),
        }
    }
}
