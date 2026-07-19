use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};
use orbit_common::types::{Workspace, WorkspaceStatus};
use orbit_store::sqlite::task_registry::{WorkspaceConfig, write_workspace_config};

use crate::OrbitRuntime;
use crate::command::activity::seed_default_activities;
use crate::command::job::seed_default_jobs;
use crate::execution_environment::reject_execution_profile_env_overrides_from;

#[test]
fn compatibility_module_reexports_host_registry_service() {
    let _service =
        super::HostRegistryService::new(orbit_store::Store::open_in_memory().expect("store"));
}

fn workspace(id: &str, owner_machine_id: Option<&str>) -> Workspace {
    let now = Utc
        .with_ymd_and_hms(2026, 7, 18, 8, 0, 0)
        .single()
        .expect("timestamp");
    Workspace {
        id: id.to_string(),
        name: id.to_string(),
        owner_machine_id: owner_machine_id.map(ToOwned::to_owned),
        git_remote: Some("git@github.com:example/repo.git".to_string()),
        ship_mode: Some("pr".to_string()),
        base_branch: "agent-main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: now,
        updated_at: now,
    }
}

const PROFILE_CONFIG: &str = r#"
[workflow]
base_branch = "agent-main"
default_crew = "sol"

[crews.sol]
model = "gpt-test"
provider = "openai"
backend = "cli"
description = "  Systems implementation  "
tags = ["review", " hard ", "review", ""]

[crews.qa]
model = "claude-test"
provider = "claude"
backend = "cli"
"#;

const PROFILE_WORKSPACE_ID: &str = "alpha-abc123";

fn profile_runtime(config: &str) -> (tempfile::TempDir, OrbitRuntime, PathBuf, PathBuf, Workspace) {
    let root = tempfile::tempdir().expect("tempdir");
    let global_root = root.path().join("global");
    let repo_root = root.path().join("repo");
    let workspace_root = repo_root.join(".orbit");
    fs::create_dir_all(&global_root).expect("global root");
    fs::create_dir_all(&workspace_root).expect("workspace root");
    write_workspace_config(
        &workspace_root,
        &WorkspaceConfig {
            schema_version: 1,
            workspace_id: PROFILE_WORKSPACE_ID.to_string(),
        },
    )
    .expect("workspace config");
    fs::write(workspace_root.join("config.toml"), config).expect("runtime config");
    seed_default_activities(&global_root.join("resources/activities"), true)
        .expect("seed activities");
    seed_default_jobs(&global_root.join("resources/jobs"), true).expect("seed jobs");
    let runtime = OrbitRuntime::from_roots(&global_root, &workspace_root).expect("runtime");
    (
        root,
        runtime,
        global_root,
        workspace_root,
        workspace(PROFILE_WORKSPACE_ID, Some("hm_owner")),
    )
}

fn build_profile(runtime: &OrbitRuntime, workspace: &Workspace) -> super::ExecutionProfileV1 {
    runtime
        .build_execution_profile_v1(
            workspace,
            "hm_owner",
            Utc.with_ymd_and_hms(2026, 7, 18, 9, 0, 0)
                .single()
                .expect("timestamp"),
        )
        .expect("build profile")
}

#[test]
fn execution_environment_snapshot_contains_only_core_derived_facts() {
    let (_root, runtime, _global, _workspace_root, _workspace) = profile_runtime(PROFILE_CONFIG);

    let snapshot = runtime
        .execution_environment_snapshot()
        .expect("execution environment snapshot");

    assert_eq!(snapshot.workspace_id, PROFILE_WORKSPACE_ID);
    assert_eq!(snapshot.workflow_base_branch, "agent-main");
    assert_eq!(snapshot.crews.default_crew.as_deref(), Some("sol"));
    assert!(snapshot.crews.crews.iter().any(|crew| crew.name == "sol"));
    assert_eq!(snapshot.ship_closure_digest.len(), 64);
}

fn append_comment(path: &Path) {
    let mut body = fs::read_to_string(path).expect("read asset");
    body.push_str("\n# formatting-only profile fixture\n");
    fs::write(path, body).expect("write commented asset");
}

#[test]
fn execution_profile_payload_is_frozen_normalized_and_path_format_order_stable() {
    let (_root_a, runtime_a, global_a, _workspace_a, workspace_a) = profile_runtime(PROFILE_CONFIG);
    let profile_a = build_profile(&runtime_a, &workspace_a);
    assert_eq!(profile_a.crews[0].name, "qa");
    let sol = profile_a
        .crews
        .iter()
        .find(|crew| crew.name == "sol")
        .expect("sol crew");
    assert_eq!(sol.provider, "codex", "provider alias canonicalizes");
    assert_eq!(sol.description.as_deref(), Some("Systems implementation"));
    assert_eq!(sol.tags, vec!["hard", "review"]);

    let payload = serde_json::to_value(&profile_a).expect("profile JSON");
    let keys = payload
        .as_object()
        .expect("profile object")
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        keys,
        BTreeSet::from([
            "schema_version",
            "workspace_id",
            "owner_machine_id",
            "observed_at",
            "config_digest",
            "default_crew",
            "crews",
            "ship",
        ])
    );
    assert!(payload.get("generation").is_none());
    assert!(payload.get("received_at").is_none());
    assert_eq!(
        payload["ship"]
            .as_object()
            .expect("ship object")
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["mode", "base_branch", "ship_closure_digest"])
    );

    append_comment(
        &global_a
            .join("resources/jobs")
            .join("task_auto_pipeline.yaml"),
    );
    let formatting_only = build_profile(&runtime_a, &workspace_a);
    assert_eq!(
        formatting_only.ship.ship_closure_digest,
        profile_a.ship.ship_closure_digest
    );

    let reordered = r#"
[workflow]
base_branch = "agent-main"
default_crew = "sol"

[crews.qa]
model = "claude-test"
provider = "claude"
backend = "cli"

[crews.sol]
tags = ["", "review", " hard ", "review"]
description = "  Systems implementation  "
backend = "cli"
provider = "openai"
model = "gpt-test"
"#;
    let (_root_b, runtime_b, _global_b, _workspace_b, workspace_b) = profile_runtime(reordered);
    let profile_b = build_profile(&runtime_b, &workspace_b);
    assert_eq!(profile_b.config_digest, profile_a.config_digest);
    assert_eq!(
        profile_b.ship.ship_closure_digest,
        profile_a.ship.ship_closure_digest
    );
}

#[test]
fn closure_digest_tracks_execution_precedence_jobs_activities_recovery_and_ignores_unrelated_assets()
 {
    let (_root, runtime, global_root, workspace_root, workspace) = profile_runtime(PROFILE_CONFIG);
    let baseline = build_profile(&runtime, &workspace);

    // List/show precedence prefers workspace assets, but name-based execution
    // keeps shipped global defaults authoritative for these fixed job names.
    let local_job = workspace_root.join("resources/jobs/task_local_pipeline.yaml");
    fs::create_dir_all(local_job.parent().expect("job parent")).expect("job parent");
    fs::write(
        &local_job,
        "schemaVersion: 2\nkind: Job\nmetadata: { name: task_local_pipeline }\nspec: { state: enabled, steps: [] }\n",
    )
    .expect("workspace shadow");
    let workspace_shadow = build_profile(&runtime, &workspace);
    assert_eq!(
        workspace_shadow.ship.ship_closure_digest,
        baseline.ship.ship_closure_digest
    );

    append_comment(
        &global_root
            .join("resources/jobs")
            .join("workspace_ship_pipeline.yaml"),
    );
    fs::write(
        global_root.join("resources/jobs/unrelated.yaml"),
        "schemaVersion: 2\nkind: Job\nmetadata: { name: unrelated }\nspec: { state: enabled, steps: [] }\n",
    )
    .expect("unrelated job");
    fs::write(
        global_root.join("resources/activities/unrelated.yaml"),
        "schemaVersion: 2\nkind: Activity\nmetadata: { name: unrelated }\nspec: { type: deterministic, description: unrelated, action: unrelated }\n",
    )
    .expect("unrelated activity");
    let unrelated = build_profile(&runtime, &workspace);
    assert_eq!(
        unrelated.ship.ship_closure_digest,
        baseline.ship.ship_closure_digest
    );

    let local_path = global_root.join("resources/jobs/task_local_pipeline.yaml");
    let local_body = fs::read_to_string(&local_path).expect("local job");
    fs::write(
        &local_path,
        local_body.replacen(
            "recovery_activity: step_failure_recovery",
            "recovery_activity: agent_implement",
            1,
        ),
    )
    .expect("mutate recovery binding");
    let recovery_binding_changed = build_profile(&runtime, &workspace);
    assert_ne!(
        recovery_binding_changed.ship.ship_closure_digest,
        baseline.ship.ship_closure_digest
    );
    fs::write(&local_path, &local_body).expect("restore recovery binding");

    fs::write(
        &local_path,
        local_body.replacen("max_active_runs: 10", "max_active_runs: 9", 1),
    )
    .expect("mutate selected job");
    let job_changed = build_profile(&runtime, &workspace);
    assert_ne!(
        job_changed.ship.ship_closure_digest,
        baseline.ship.ship_closure_digest
    );
    fs::write(&local_path, local_body).expect("restore selected job");

    let recovery_path = global_root.join("resources/activities/step_failure_recovery.yaml");
    let recovery_body = fs::read_to_string(&recovery_path).expect("recovery activity");
    fs::write(
        &recovery_path,
        recovery_body.replacen(
            "Generic v2 task workflow recovery hook.",
            "Changed recovery semantics for digest fixture.",
            1,
        ),
    )
    .expect("mutate recovery");
    let recovery_changed = build_profile(&runtime, &workspace);
    assert_ne!(
        recovery_changed.ship.ship_closure_digest,
        baseline.ship.ship_closure_digest
    );

    let auto_body =
        recovery_body.replacen("  role: reviewer", "  backend: auto\n  role: reviewer", 1);
    fs::write(&recovery_path, &auto_body).expect("use backend auto");
    let auto_resolved = build_profile(&runtime, &workspace);
    assert_eq!(
        auto_resolved.ship.ship_closure_digest, baseline.ship.ship_closure_digest,
        "backend:auto must hash its concrete execution result"
    );
    fs::write(
        &recovery_path,
        auto_body.replacen("backend: auto", "backend: http", 1),
    )
    .expect("use concrete alternate backend");
    let backend_changed = build_profile(&runtime, &workspace);
    assert_ne!(
        backend_changed.ship.ship_closure_digest,
        baseline.ship.ship_closure_digest
    );
    fs::write(
        &recovery_path,
        auto_body.replacen("backend: auto", "backend: ambiguous", 1),
    )
    .expect("use unknown backend");
    let backend_error = runtime
        .build_execution_profile_v1(&workspace, "hm_owner", Utc::now())
        .expect_err("unknown backend fails profile publication")
        .to_string();
    assert!(backend_error.contains("backend") || backend_error.contains("ambiguous"));
}

#[test]
fn config_digest_tracks_crew_mode_and_base_while_closure_is_independent() {
    let (_root, runtime, _global, _workspace, workspace) = profile_runtime(PROFILE_CONFIG);
    let baseline = build_profile(&runtime, &workspace);

    let changed_crew_config = PROFILE_CONFIG.replace("model = \"gpt-test\"", "model = \"gpt-new\"");
    let (_crew_root, crew_runtime, _crew_global, _crew_workspace, crew_ws) =
        profile_runtime(&changed_crew_config);
    let crew_changed = build_profile(&crew_runtime, &crew_ws);
    assert_ne!(crew_changed.config_digest, baseline.config_digest);
    assert_eq!(
        crew_changed.ship.ship_closure_digest,
        baseline.ship.ship_closure_digest
    );

    let mut local_workspace = workspace.clone();
    local_workspace.ship_mode = Some("local".to_string());
    let mode_changed = build_profile(&runtime, &local_workspace);
    assert_ne!(mode_changed.config_digest, baseline.config_digest);
    assert_eq!(
        mode_changed.ship.ship_closure_digest,
        baseline.ship.ship_closure_digest
    );

    let mismatch_config = PROFILE_CONFIG.replace("agent-main", "main");
    let (_base_root, base_runtime, _base_global, _base_workspace, mut base_ws) =
        profile_runtime(&mismatch_config);
    let mismatch = base_runtime
        .build_execution_profile_v1(&base_ws, "hm_owner", Utc::now())
        .expect_err("base mismatch fails")
        .to_string();
    assert!(mismatch.contains("does not match runtime"));
    base_ws.base_branch = "main".to_string();
    let base_changed = build_profile(&base_runtime, &base_ws);
    assert_ne!(base_changed.config_digest, baseline.config_digest);
    assert_eq!(
        base_changed.ship.ship_closure_digest,
        baseline.ship.ship_closure_digest
    );
}

#[test]
fn unsupported_execution_environment_overrides_fail_closed_without_values() {
    let error = reject_execution_profile_env_overrides_from(|name| {
        (name == "ORBIT_JOB_DIR").then(|| "/secret/catalog/path".to_string())
    })
    .expect_err("override must fail")
    .to_string();
    assert!(error.contains("ORBIT_JOB_DIR"));
    assert!(!error.contains("/secret/catalog/path"));
}
