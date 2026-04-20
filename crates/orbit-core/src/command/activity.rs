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
        "git_push",
        include_str!("../../assets/activities/git_push.yaml"),
    ),
    (
        "invoke_and_wait",
        include_str!("../../assets/activities/invoke_and_wait.yaml"),
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
    ("sleep", include_str!("../../assets/activities/sleep.yaml")),
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
