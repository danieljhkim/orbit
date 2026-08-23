//! Default routine seeding [ORB-10129].
//!
//! Routines are workspace-authored YAML under `.orbit/routines/` — unlike
//! activities and jobs there is no global routines directory, so defaults
//! are seeded per workspace on `orbit init`. Two placeholders are resolved
//! at seed time:
//!
//! - `__ORBIT_HOST_ID__` — routines v1 has no "any host", so the seeded
//!   definition pins the host identity supplied by the feature layer.
//! - `__ORBIT_ROUTINE_NAME__` — routine names must be unique across all
//!   routine sources on a host, so the seeded name carries a
//!   workspace-derived suffix (`task-triage-<workspace>`) to keep two
//!   seeded source workspaces from colliding fail-closed.
//!
//! Seeded routines are inert until the workspace opts into
//! `[routines] role = "source"`; they exist so a fresh workspace gets
//! reviewable, opt-in schedules without silently enabling unattended work.

use std::borrow::Cow;
use std::path::Path;

use orbit_common::OrbitError;
use orbit_common::protocol::yaml::parse_routine_yaml;

use super::{ManagedAssetLayout, ManagedAssetReconciliation, reconcile_managed_assets};

/// Shippable default routine assets, seeded under
/// `<workspace>/.orbit/routines/<file>.yaml` on `orbit init`. Every entry
/// must keep the `__ORBIT_HOST_ID__` / `__ORBIT_ROUTINE_NAME__`
/// placeholders parseable once substituted — `seed_default_routines`
/// validates each rendered document fail-closed before writing.
pub(crate) const DEFAULT_ROUTINE_FILES: &[(&str, &str)] = &[
    (
        "auto_task_scheduler",
        include_str!("../../assets/routines/auto_task_scheduler.yaml"),
    ),
    (
        "task_triage",
        include_str!("../../assets/routines/task_triage.yaml"),
    ),
    (
        "task_pilot",
        include_str!("../../assets/routines/task_pilot.yaml"),
    ),
    (
        "ship_sweep",
        include_str!("../../assets/routines/ship_sweep.yaml"),
    ),
    (
        "worktree_gc",
        include_str!("../../assets/routines/worktree_gc.yaml"),
    ),
];

const HOST_ID_PLACEHOLDER: &str = "__ORBIT_HOST_ID__";
const ROUTINE_NAME_PLACEHOLDER: &str = "__ORBIT_ROUTINE_NAME__";

/// Seed every entry in [`DEFAULT_ROUTINE_FILES`] under `routines_dir`,
/// resolving the host and routine-name placeholders. Mirrors the activity /
/// job seeding convention: when `overwrite` is false (plain re-init),
/// existing files are preserved. Destructive initialization may set
/// `overwrite`, though `--force` normally recreates the whole root first.
///
/// Seeding is manifest-aware: the recorded digest is taken over the *rendered*
/// document — after host-id and routine-name substitution — because that is
/// what actually lands on disk. A default dropped from a later release is
/// therefore retired by content provenance, and a re-seed of unchanged
/// embedded content is a no-op rather than a rewrite.
// ADR-0215: default routines are seeded per workspace with host and name
// resolved at seed time — routines have no global directory and v1 requires
// explicit host pinning and host-unique names.
// ADR-0366: the recorded digest covers the rendered document, so a
// placeholder-substituting asset still gets honest provenance.
pub(crate) fn seed_default_routines(
    routines_dir: &Path,
    host_id: &str,
    workspace_slug: Option<&str>,
    overwrite: bool,
) -> Result<ManagedAssetReconciliation, OrbitError> {
    let host_id = host_id.trim();
    if host_id.is_empty() {
        return Err(OrbitError::InvalidInput(
            "cannot seed default routines without a host id".to_string(),
        ));
    }
    reconcile_managed_assets(
        routines_dir,
        "routine",
        ManagedAssetLayout::YamlStem,
        DEFAULT_ROUTINE_FILES,
        overwrite,
        |file_stem, template| {
            let routine_name = routine_name_for(file_stem, workspace_slug);
            let rendered = template
                .replace(ROUTINE_NAME_PLACEHOLDER, &routine_name)
                .replace(HOST_ID_PLACEHOLDER, host_id);
            // Fail-closed: never seed a file the routine loader would reject.
            parse_routine_yaml(&rendered).map_err(|error| {
                OrbitError::InvalidInput(format!(
                    "default routine `{file_stem}` failed validation after placeholder \
                     substitution (host `{host_id}`, name `{routine_name}`): {error}"
                ))
            })?;
            Ok(Cow::Owned(rendered))
        },
    )
}

/// Compose a per-workspace routine name: `<stem>-<workspace-slug>`, using
/// the routine name charset (lowercase alphanumeric plus `-`/`_`, starting
/// alphanumeric). Names must be unique across all routine sources on a
/// host, so the workspace suffix is what lets two seeded sources coexist.
fn routine_name_for(file_stem: &str, workspace_slug: Option<&str>) -> String {
    let base = file_stem.replace('_', "-");
    match workspace_slug.map(sanitize_routine_name_part) {
        Some(slug) if !slug.is_empty() => format!("{base}-{slug}"),
        _ => base,
    }
}

fn sanitize_routine_name_part(raw: &str) -> String {
    let lowered = raw.trim().to_ascii_lowercase();
    let mut out = String::with_capacity(lowered.len());
    for ch in lowered.chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    out.trim_matches(|ch| ch == '-' || ch == '_').to_string()
}

#[cfg(test)]
mod tests {
    use orbit_types::workflow::{OverlapPolicy, RoutineTarget};
    use tempfile::tempdir;

    use crate::routines::parse_cron;

    use super::*;

    #[test]
    fn seeded_routines_are_valid_disabled_pinned_and_workspace_unique() {
        let root = tempdir().expect("create tempdir");
        let routines_dir = root.path().join(".orbit/routines");
        let seeded = seed_default_routines(&routines_dir, "test-host", Some("My Repo!"), true)
            .expect("seed default routines");
        assert_eq!(seeded.refreshed, DEFAULT_ROUTINE_FILES.len());

        for (stem, target) in [
            ("auto_task_scheduler", "auto_task_scheduler_pipeline"),
            ("task_triage", "task_triage_pipeline"),
            ("task_pilot", "task_pilot_pipeline"),
            ("ship_sweep", "workspace_ship_pipeline"),
            ("worktree_gc", "worktree_gc_pipeline"),
        ] {
            let yaml = std::fs::read_to_string(routines_dir.join(format!("{stem}.yaml")))
                .expect("read seeded routine");
            let definition = parse_routine_yaml(&yaml).expect("seeded routine parses fail-closed");
            assert_eq!(
                definition.name,
                format!("{}-my-repo", stem.replace('_', "-"))
            );
            assert_eq!(definition.hosts, vec!["test-host".to_string()]);
            assert_eq!(definition.target, RoutineTarget::Job(target.to_string()));
            assert_eq!(definition.policy.overlap, OverlapPolicy::Forbid);
            assert!(!definition.enabled);
        }

        // Cadence: every 40 minutes — deliberately sparser than the
        // ~20-minute ship sweep, and parseable by the scheduler.
        let triage = std::fs::read_to_string(routines_dir.join("task_triage.yaml"))
            .expect("read triage routine");
        let triage = parse_routine_yaml(&triage).expect("triage routine parses");
        assert_eq!(triage.trigger.cron, "15 * * * *");
        parse_cron(&triage.trigger.cron).expect("seeded cron parses");

        // Task-pilot may run up to ten five-task partitions in two waves,
        // each agent bounded to 30 minutes. Its 90-minute timeout covers
        // that maximum automatic batch plus deterministic preparation/apply.
        let pilot = std::fs::read_to_string(routines_dir.join("task_pilot.yaml"))
            .expect("read task-pilot routine");
        let pilot = parse_routine_yaml(&pilot).expect("task-pilot routine parses");
        assert_eq!(pilot.trigger.cron, "*/40 * * * *");
        assert_eq!(
            pilot.trigger.missed_run,
            orbit_types::workflow::MissedRunPolicy::Skip
        );
        assert_eq!(pilot.policy.timeout_minutes, 90);
        assert_eq!(pilot.policy.overlap, OverlapPolicy::Forbid);
        parse_cron(&pilot.trigger.cron).expect("task-pilot cron parses");

        let ship = std::fs::read_to_string(routines_dir.join("ship_sweep.yaml"))
            .expect("read ship routine");
        let ship = parse_routine_yaml(&ship).expect("ship routine parses");
        assert_eq!(
            ship.trigger.missed_run,
            orbit_types::workflow::MissedRunPolicy::Skip
        );
        assert_eq!(ship.trigger.cron, "*/20 * * * *");
        parse_cron(&ship.trigger.cron).expect("ship cron parses");

        let gc = std::fs::read_to_string(routines_dir.join("worktree_gc.yaml"))
            .expect("read worktree GC routine");
        let gc = parse_routine_yaml(&gc).expect("worktree GC routine parses");
        assert!(!gc.enabled);
        assert_eq!(gc.policy.overlap, OverlapPolicy::Forbid);
        assert_eq!(gc.trigger.cron, "35 * * * *");
    }

    #[test]
    fn seeding_preserves_existing_files_unless_overwrite() {
        let root = tempdir().expect("create tempdir");
        let routines_dir = root.path().join("routines");
        seed_default_routines(&routines_dir, "host-a", None, false).expect("first seed");
        let path = routines_dir.join("worktree_gc.yaml");
        std::fs::write(&path, "user edited").expect("simulate user edit");

        let seeded = seed_default_routines(&routines_dir, "host-a", None, false).expect("re-seed");
        assert_eq!(seeded.refreshed, 0);
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "user edited",
            "plain re-init must not clobber user edits"
        );

        seed_default_routines(&routines_dir, "host-b", None, true).expect("refresh defaults");
        let refreshed = std::fs::read_to_string(&path).expect("read refreshed");
        assert!(refreshed.contains("host-b"));
        let definition = parse_routine_yaml(&refreshed).expect("refreshed routine parses");
        assert_eq!(definition.name, "worktree-gc");
        assert_eq!(
            definition.target,
            RoutineTarget::Job("worktree_gc_pipeline".to_string())
        );
        assert!(!definition.enabled);
    }

    #[test]
    fn plain_reinit_adds_a_new_missing_default_without_rewriting_existing_files() {
        const HOST_ID: &str = "host-a";

        let root = tempdir().expect("create tempdir");
        let routines_dir = root.path().join("routines");
        seed_default_routines(&routines_dir, HOST_ID, Some("workspace"), false)
            .expect("first seed");

        let existing = routines_dir.join("task_triage.yaml");
        let original = std::fs::read(&existing).expect("read existing routine bytes");
        let missing = routines_dir.join("task_pilot.yaml");
        std::fs::remove_file(&missing).expect("remove newly introduced routine");

        let seeded = seed_default_routines(&routines_dir, HOST_ID, Some("workspace"), false)
            .expect("plain re-init");
        assert_eq!(seeded.refreshed, 1, "only the missing default is created");
        assert_eq!(
            std::fs::read(&existing).expect("read existing routine bytes"),
            original,
            "plain re-init must preserve existing routines byte-for-byte"
        );

        let pilot = parse_routine_yaml(
            &std::fs::read_to_string(&missing).expect("read newly seeded task-pilot routine"),
        )
        .expect("newly seeded task-pilot routine parses");
        assert_eq!(pilot.hosts, vec![HOST_ID.to_string()]);
        assert!(!pilot.enabled);
    }

    /// The recorded digest covers the *rendered* document, so re-seeding
    /// unchanged embedded content against the same host and workspace must not
    /// rewrite a single file — even under `overwrite`. A steady-state
    /// bootstrap can then run against a read-only routines directory.
    #[test]
    fn reseeding_unchanged_rendered_content_is_a_no_op_not_a_rewrite() {
        let root = tempdir().expect("create tempdir");
        let routines_dir = root.path().join("routines");
        seed_default_routines(&routines_dir, "host-a", Some("workspace"), true)
            .expect("first seed");

        let before: Vec<(std::path::PathBuf, std::time::SystemTime)> = DEFAULT_ROUTINE_FILES
            .iter()
            .map(|(stem, _)| {
                let path = routines_dir.join(format!("{stem}.yaml"));
                let modified = std::fs::metadata(&path)
                    .and_then(|metadata| metadata.modified())
                    .expect("read seeded routine mtime");
                (path, modified)
            })
            .collect();

        let reseeded = seed_default_routines(&routines_dir, "host-a", Some("workspace"), true)
            .expect("re-seed unchanged rendered content");
        assert_eq!(reseeded.refreshed, 0, "unchanged routines must not rewrite");
        assert_eq!(reseeded.retired, 0);
        assert!(reseeded.warnings.is_empty());

        for (path, modified) in before {
            let current = std::fs::metadata(&path)
                .and_then(|metadata| metadata.modified())
                .expect("read routine mtime after re-seed");
            assert_eq!(
                current,
                modified,
                "re-seed rewrote `{}` despite identical rendered content",
                path.display()
            );
        }

        // A different host renders different content, so the digest changes
        // and an overwriting seed does refresh every file.
        let rehosted = seed_default_routines(&routines_dir, "host-b", Some("workspace"), true)
            .expect("re-seed with a new host id");
        assert_eq!(rehosted.refreshed, DEFAULT_ROUTINE_FILES.len());
    }

    /// Rewrite the top-level `enabled:` line to a fixed marker so a workspace's
    /// own opt-in decision does not read as template drift.
    fn ignoring_enabled(routine: &str) -> String {
        routine
            .lines()
            .map(|line| {
                if line.starts_with("enabled:") {
                    "enabled: <workspace decision>"
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// This workspace's own routine may differ from the template in exactly one
    /// field. The asset documents flipping `enabled` as "an explicit, versioned
    /// workspace decision", so comparing it too would fail the moment a
    /// workspace does the thing the template invites. Everything else — cadence,
    /// host pin, target, policy, the rationale comments — must stay aligned.
    #[test]
    fn dogfood_task_pilot_routine_matches_the_rendered_default_template() {
        let rendered = include_str!("../../assets/routines/task_pilot.yaml")
            .replace(ROUTINE_NAME_PLACEHOLDER, "task-pilot-orbit")
            .replace(HOST_ID_PLACEHOLDER, "dk-server-1");
        assert_eq!(
            ignoring_enabled(&rendered),
            ignoring_enabled(include_str!("../../../../.orbit/routines/task_pilot.yaml")),
            "dogfood task-pilot routine must stay aligned with the seeded template outside `enabled`"
        );
    }

    #[test]
    fn seeding_requires_a_host_id() {
        let root = tempdir().expect("create tempdir");
        let err = seed_default_routines(&root.path().join("routines"), "  ", None, true)
            .expect_err("empty host id must not seed an unloadable routine");
        assert!(err.to_string().contains("host id"), "{err}");
    }
}
