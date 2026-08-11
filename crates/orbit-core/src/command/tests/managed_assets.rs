use std::path::Path;

use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::tempdir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use super::super::MANAGED_ASSET_MANIFEST_FILE;
#[cfg(unix)]
use super::super::activity::seed_default_activities;
use super::super::init::{InitOptions, InitResult, init_workspace_at_root};
#[cfg(unix)]
use super::super::job::seed_default_jobs;
use crate::OrbitRuntime;
use crate::command::job::JobCatalogFilter;
use crate::config::agent_detect::DetectedAgents;

fn init_global(root: &Path) -> InitResult {
    init_workspace_at_root(
        root,
        InitOptions {
            global_only: true,
            refresh_defaults: true,
            detected: Some(DetectedAgents::default()),
            ..Default::default()
        },
    )
    .expect("initialize global defaults")
}

fn sha256(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

fn add_managed_manifest_entry(dir: &Path, name: &str, content: &str) {
    let manifest_path = dir.join(MANAGED_ASSET_MANIFEST_FILE);
    let raw = std::fs::read_to_string(&manifest_path).expect("read managed manifest");
    let mut manifest: Value = serde_json::from_str(&raw).expect("parse managed manifest");
    manifest["assets"]
        .as_object_mut()
        .expect("manifest assets object")
        .insert(name.to_string(), Value::String(sha256(content)));
    std::fs::write(
        manifest_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&manifest).expect("serialize managed manifest")
        ),
    )
    .expect("write previous managed manifest");
}

fn activity_yaml(name: &str, tool: Option<&str>) -> String {
    match tool {
        Some(tool) => format!(
            r#"schemaVersion: 2
kind: Activity
metadata:
  name: {name}
spec:
  type: agent_loop
  description: retired test activity
  input_schema_json: {{type: object}}
  output_schema_json: {{type: object}}
  instruction: retired
  tools: [{tool}]
  max_iterations: 1
"#
        ),
        None => format!(
            r#"schemaVersion: 2
kind: Activity
metadata:
  name: {name}
spec:
  type: deterministic
  description: operator activity
  input_schema_json: {{type: object}}
  output_schema_json: {{type: object}}
  action: sleep
  config: {{}}
"#
        ),
    }
}

fn job_yaml(name: &str, action: Option<&str>) -> String {
    let steps = action.map_or_else(
        || "  steps: []\n".to_string(),
        |action| {
            format!(
                r#"  steps:
    - id: removed
      spec:
        type: deterministic
        action: {action}
        config: {{}}
"#
            )
        },
    );
    format!(
        r#"schemaVersion: 2
kind: Job
metadata:
  name: {name}
spec:
  state: enabled
  kind: workflow
  max_active_runs: 1
{steps}"#
    )
}

#[cfg(unix)]
fn make_read_only(path: &Path) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o555))
        .expect("make managed asset directory read-only");
}

#[cfg(unix)]
fn make_writable(path: &Path) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("restore managed asset directory permissions");
}

#[cfg(unix)]
#[test]
fn steady_state_reconcile_skips_asset_and_manifest_writes() {
    let root = tempdir().expect("create tempdir");
    let global_root = root.path().join("global");
    init_global(&global_root);

    let activities_dir = global_root.join("resources/activities");
    let jobs_dir = global_root.join("resources/jobs");
    make_read_only(&activities_dir);
    make_read_only(&jobs_dir);

    let activities = seed_default_activities(&activities_dir, true);
    let jobs = seed_default_jobs(&jobs_dir, true);

    make_writable(&activities_dir);
    make_writable(&jobs_dir);

    let activities = activities.expect("unchanged activities do not require write access");
    let jobs = jobs.expect("unchanged jobs do not require write access");

    assert_eq!(activities.refreshed, 0);
    assert_eq!(activities.retired, 0);
    assert!(activities.warnings.is_empty());
    assert_eq!(jobs.refreshed, 0);
    assert_eq!(jobs.retired, 0);
    assert!(jobs.warnings.is_empty());
}

#[cfg(unix)]
#[test]
fn runtime_bootstrap_reads_seeded_global_assets_without_write_access() {
    let root = tempdir().expect("create tempdir");
    let global_root = root.path().join("global");
    let workspace_root = root.path().join("repo/.orbit");
    init_global(&global_root);

    let activities_dir = global_root.join("resources/activities");
    let jobs_dir = global_root.join("resources/jobs");
    make_read_only(&activities_dir);
    make_read_only(&jobs_dir);

    let result = OrbitRuntime::from_roots(&global_root, &workspace_root)
        .and_then(|runtime| runtime.list_job_catalog_with_last_run(true, JobCatalogFilter::All));

    make_writable(&activities_dir);
    make_writable(&jobs_dir);

    result.expect("ordinary read-only runtime operation succeeds");
}

#[test]
fn refresh_retires_managed_activity_and_job_assets_and_is_idempotent() {
    let root = tempdir().expect("create tempdir");
    let global_root = root.path().join("global");
    let workspace_root = root.path().join("repo/.orbit");
    init_global(&global_root);

    let activities_dir = global_root.join("resources/activities");
    let jobs_dir = global_root.join("resources/jobs");
    let clean_activity_name = "retired_activity_clean";
    let modified_activity_name = "retired_activity_modified";
    let clean_job_name = "retired_job_clean";
    let modified_job_name = "retired_job_modified";
    let clean_activity = activity_yaml(clean_activity_name, Some("removed.tool"));
    let original_modified_activity = activity_yaml(modified_activity_name, Some("removed.tool"));
    let clean_job = job_yaml(clean_job_name, Some("removed_action"));
    let original_modified_job = job_yaml(modified_job_name, Some("removed_action"));

    for (dir, name, content) in [
        (
            &activities_dir,
            clean_activity_name,
            clean_activity.as_str(),
        ),
        (
            &activities_dir,
            modified_activity_name,
            original_modified_activity.as_str(),
        ),
        (&jobs_dir, clean_job_name, clean_job.as_str()),
        (&jobs_dir, modified_job_name, original_modified_job.as_str()),
    ] {
        add_managed_manifest_entry(dir, name, content);
        std::fs::write(dir.join(format!("{name}.yaml")), content)
            .expect("materialize previous managed asset");
    }

    let modified_activity = activity_yaml(modified_activity_name, Some("still.removed"));
    let modified_job = job_yaml(modified_job_name, Some("still_removed"));
    std::fs::write(
        activities_dir.join(format!("{modified_activity_name}.yaml")),
        &modified_activity,
    )
    .expect("locally modify retired activity");
    std::fs::write(
        jobs_dir.join(format!("{modified_job_name}.yaml")),
        &modified_job,
    )
    .expect("locally modify retired job");

    let reconciled = init_global(&global_root);
    assert_eq!(reconciled.retired_default_activities, 2);
    assert_eq!(reconciled.retired_default_jobs, 2);
    assert_eq!(reconciled.managed_asset_warnings.len(), 2);
    for warning in &reconciled.managed_asset_warnings {
        assert!(warning.contains("locally modified"));
        assert!(warning.contains("preserved"));
        assert!(warning.contains("migrate"));
    }

    assert!(
        !activities_dir
            .join(format!("{clean_activity_name}.yaml"))
            .exists()
    );
    assert!(!jobs_dir.join(format!("{clean_job_name}.yaml")).exists());
    assert_eq!(
        std::fs::read_to_string(global_root.join(format!(
            "resources/.retired-managed/activities/{modified_activity_name}.yaml"
        )))
        .expect("read preserved modified activity"),
        modified_activity
    );
    assert_eq!(
        std::fs::read_to_string(global_root.join(format!(
            "resources/.retired-managed/jobs/{modified_job_name}.yaml"
        )))
        .expect("read preserved modified job"),
        modified_job
    );

    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build reconciled runtime");
    let activity_catalog = runtime
        .v2_activity_catalog()
        .expect("load activity catalog");
    for retired in [clean_activity_name, modified_activity_name] {
        assert!(
            !activity_catalog.names().any(|name| name == retired),
            "retired activity `{retired}` must not remain active"
        );
    }
    let jobs = runtime
        .list_job_catalog_with_last_run(true, JobCatalogFilter::All)
        .expect("list reconciled jobs");
    for retired in [clean_job_name, modified_job_name] {
        assert!(
            !jobs.iter().any(|(entry, _)| entry.job_id == retired),
            "retired job `{retired}` must not remain active"
        );
    }

    let repeated = init_global(&global_root);
    assert_eq!(repeated.retired_default_activities, 0);
    assert_eq!(repeated.retired_default_jobs, 0);
    assert!(repeated.managed_asset_warnings.is_empty());
}

#[test]
fn first_manifest_preserves_user_assets_and_warns_about_ambiguous_legacy_yaml() {
    let root = tempdir().expect("create tempdir");
    let global_root = root.path().join("global");
    let workspace_root = root.path().join("repo/.orbit");
    let activities_dir = global_root.join("resources/activities");
    let jobs_dir = global_root.join("resources/jobs");
    std::fs::create_dir_all(&activities_dir).expect("create legacy activity dir");
    std::fs::create_dir_all(&jobs_dir).expect("create legacy job dir");

    let user_activity_name = "operator_activity";
    let user_job_name = "operator_job";
    let user_activity = activity_yaml(user_activity_name, None);
    let user_job = job_yaml(user_job_name, None);
    let colliding_activity = activity_yaml("sleep", None);
    let colliding_job = job_yaml("worktree_gc_pipeline", None);
    std::fs::write(
        activities_dir.join(format!("{user_activity_name}.yaml")),
        &user_activity,
    )
    .expect("write user activity");
    std::fs::write(jobs_dir.join(format!("{user_job_name}.yaml")), &user_job)
        .expect("write user job");
    std::fs::write(activities_dir.join("sleep.yaml"), &colliding_activity)
        .expect("write user activity colliding with a bundled name");
    std::fs::write(jobs_dir.join("worktree_gc_pipeline.yaml"), &colliding_job)
        .expect("write user job colliding with a bundled name");

    let migrated = init_global(&global_root);
    assert_eq!(migrated.managed_asset_warnings.len(), 2);
    for warning in &migrated.managed_asset_warnings {
        assert!(warning.contains("no managed provenance"));
        assert!(warning.contains("preserved in place"));
        assert!(warning.contains("move or delete"));
    }
    assert_eq!(
        std::fs::read_to_string(activities_dir.join(format!("{user_activity_name}.yaml")))
            .expect("read preserved user activity"),
        user_activity
    );
    assert_eq!(
        std::fs::read_to_string(jobs_dir.join(format!("{user_job_name}.yaml")))
            .expect("read preserved user job"),
        user_job
    );
    assert_eq!(
        std::fs::read_to_string(activities_dir.join("sleep.yaml"))
            .expect("read preserved colliding user activity"),
        colliding_activity
    );
    assert_eq!(
        std::fs::read_to_string(jobs_dir.join("worktree_gc_pipeline.yaml"))
            .expect("read preserved colliding user job"),
        colliding_job
    );

    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build migrated runtime");
    assert!(
        runtime
            .v2_activity_catalog()
            .expect("load activity catalog")
            .names()
            .any(|name| name == user_activity_name)
    );
    assert!(
        runtime
            .list_job_catalog_with_last_run(true, JobCatalogFilter::All)
            .expect("list jobs")
            .iter()
            .any(|(entry, _)| entry.job_id == user_job_name)
    );
}
