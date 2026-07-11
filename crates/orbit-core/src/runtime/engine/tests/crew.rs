//! Sibling tests for `crew.rs` (migrated per ORB-00246 / docs/design-patterns/test_layout.md).

use std::sync::{Mutex, MutexGuard, OnceLock};

use chrono::Utc;
use orbit_common::types::activity_job::ProviderSource;
use serde_json::json;
use tempfile::{TempDir, tempdir};

use super::super::crew::select_crew_name;
use crate::OrbitRuntime;
use crate::command::task::TaskAddParams;

const CONSTELLATION_DEFAULT_PROVIDER_ENV: &str = "CONSTELLATION_DEFAULT_PROVIDER";

/// Serialize the process-wide `CONSTELLATION_DEFAULT_PROVIDER` mutations so the
/// env-default test cannot race concurrent tests.
fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn runtime_with_named_crews() -> (TempDir, OrbitRuntime) {
    let root = tempdir().expect("create temp root");
    let global_root = root.path().join("global");
    let repo_root = root.path().join("repo");
    let workspace_root = repo_root.join(".orbit");
    std::fs::create_dir_all(&global_root).expect("create global root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::write(
        workspace_root.join("config.toml"),
        r#"
[crews.primary]
planner = { model = "default-planner", provider = "codex", backend = "cli" }
implementer = { model = "default-implementer", provider = "codex", backend = "cli" }
reviewer = { model = "default-reviewer", provider = "codex", backend = "cli" }

[crews.beta]
planner = { model = "beta-planner", provider = "codex", backend = "cli" }
implementer = { model = "beta-implementer", provider = "codex", backend = "cli" }
reviewer = { model = "beta-reviewer", provider = "codex", backend = "cli" }

[crews.gamma]
planner = { model = "gamma-planner", provider = "codex", backend = "cli" }
implementer = { model = "gamma-implementer", provider = "codex", backend = "cli" }
reviewer = { model = "gamma-reviewer", provider = "codex", backend = "cli" }

[workflow]
default_crew = "primary"
"#,
    )
    .expect("write test config");
    let runtime = OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build runtime");
    (root, runtime)
}

fn add_task_with_crew(runtime: &OrbitRuntime, crew: &str) -> String {
    runtime
        .add_task(TaskAddParams {
            title: format!("{crew} task"),
            description: "Task fixture for crew resolution.".to_string(),
            crew: Some(crew.to_string()),
            ..Default::default()
        })
        .expect("add task")
        .id
}

#[test]
fn run_input_task_ids_singleton_resolves_task_crew() {
    let (_root, runtime) = runtime_with_named_crews();
    let task_id = add_task_with_crew(&runtime, "beta");

    let crew = runtime
        .resolve_crew_for_run_input(&json!({ "task_ids": [task_id] }))
        .expect("resolve crew");

    assert_eq!(crew.name, "beta");
    assert_eq!(crew.planner.model, "beta-planner");
    assert_eq!(crew.implementer.model, "beta-implementer");
    assert_eq!(crew.reviewer.model, "beta-reviewer");
}

#[test]
fn record_run_crew_persists_singleton_task_ids_task_crew_models() {
    let (_root, runtime) = runtime_with_named_crews();
    let task_id = add_task_with_crew(&runtime, "beta");
    let input = json!({ "task_ids": [task_id] });
    let run = runtime
        .stores()
        .jobs()
        .insert_run("agent_implement", 1, Utc::now(), Some(input.clone()), None)
        .expect("insert run");

    let crew = runtime
        .record_run_crew_from_input(&run.run_id, &input)
        .expect("record crew");
    let stored = runtime.show_job_run(&run.run_id).expect("show stored run");

    assert_eq!(crew.name, "beta");
    assert_eq!(stored.resolved_crew.as_deref(), Some("beta"));
    assert_eq!(stored.planner_model.as_deref(), Some("beta-planner"));
    assert_eq!(
        stored.implementer_model.as_deref(),
        Some("beta-implementer")
    );
    assert_eq!(stored.reviewer_model.as_deref(), Some("beta-reviewer"));
}

#[test]
fn explicit_crew_override_wins_over_singleton_task_ids_task_crew() {
    let (_root, runtime) = runtime_with_named_crews();
    let task_id = add_task_with_crew(&runtime, "beta");

    let crew = runtime
        .resolve_crew_for_run_input(&json!({
            "crew": "gamma",
            "task_ids": [task_id]
        }))
        .expect("resolve crew");

    assert_eq!(crew.name, "gamma");
    assert_eq!(crew.implementer.model, "gamma-implementer");
}

#[test]
fn multi_task_ids_without_override_falls_back_to_default_crew() {
    let (_root, runtime) = runtime_with_named_crews();
    let beta_task_id = add_task_with_crew(&runtime, "beta");
    let gamma_task_id = add_task_with_crew(&runtime, "gamma");

    let crew = runtime
        .resolve_crew_for_run_input(&json!({
            "task_ids": [beta_task_id, gamma_task_id]
        }))
        .expect("resolve crew");

    assert_eq!(crew.name, "primary");
    assert_eq!(crew.implementer.model, "default-implementer");
}

/// Table-driven proof of the crew-selection precedence (ORB-10091, contract §3)
/// and of the environment-default tier. Each row fixes the higher tiers and
/// varies the `env` column (`CONSTELLATION_DEFAULT_PROVIDER`): a row that has an
/// explicit / task / workspace choice keeps its result regardless of `env`,
/// while the otherwise-defaulted rows flip when the single env setting changes.
#[test]
fn select_crew_name_precedence_and_environment_default() {
    struct Row {
        name: &'static str,
        explicit: Option<&'static str>,
        task: Option<&'static str>,
        workspace: Option<&'static str>,
        env: Option<&'static str>,
        system: Option<&'static str>,
        expect: Option<(&'static str, ProviderSource)>,
    }

    let rows = [
        // Explicit wins over every lower tier, including a set env.
        Row {
            name: "explicit beats all (env ignored)",
            explicit: Some("gamma"),
            task: Some("beta"),
            workspace: Some("primary"),
            env: Some("gemini"),
            system: Some("claude"),
            expect: Some(("gamma", ProviderSource::Explicit)),
        },
        // Task config wins over workspace + env when there is no explicit choice.
        Row {
            name: "task beats workspace+env",
            explicit: None,
            task: Some("beta"),
            workspace: Some("primary"),
            env: Some("gemini"),
            system: Some("claude"),
            expect: Some(("beta", ProviderSource::TaskConfig)),
        },
        // A configured workspace default is never overridden by the env lever.
        Row {
            name: "workspace beats env (unchanged by env)",
            explicit: None,
            task: None,
            workspace: Some("primary"),
            env: Some("gemini"),
            system: Some("claude"),
            expect: Some(("primary", ProviderSource::WorkspaceDefault)),
        },
        // Otherwise-defaulted: the env setting is the selection...
        Row {
            name: "env applies when otherwise-defaulted (claude)",
            explicit: None,
            task: None,
            workspace: None,
            env: Some("claude"),
            system: Some("claude"),
            expect: Some(("claude", ProviderSource::EnvironmentDefault)),
        },
        // ...and changing that one setting changes the resolved crew.
        Row {
            name: "env applies when otherwise-defaulted (gemini)",
            explicit: None,
            task: None,
            workspace: None,
            env: Some("gemini"),
            system: Some("claude"),
            expect: Some(("gemini", ProviderSource::EnvironmentDefault)),
        },
        // No env set: fall through to the system default.
        Row {
            name: "system default when nothing else",
            explicit: None,
            task: None,
            workspace: None,
            env: None,
            system: Some("claude"),
            expect: Some(("claude", ProviderSource::SystemDefault)),
        },
        // Whitespace at a tier is not a selection; fall through.
        Row {
            name: "whitespace at a tier is skipped",
            explicit: Some("   "),
            task: None,
            workspace: None,
            env: Some("codex"),
            system: Some("claude"),
            expect: Some(("codex", ProviderSource::EnvironmentDefault)),
        },
        // Nothing at any tier -> no selection (caller surfaces the error).
        Row {
            name: "nothing selected",
            explicit: None,
            task: None,
            workspace: None,
            env: None,
            system: None,
            expect: None,
        },
    ];

    for row in rows {
        let got = select_crew_name(row.explicit, row.task, row.workspace, row.env, row.system);
        assert_eq!(got, row.expect, "{}", row.name);
    }
}

/// End-to-end wiring: `resolve_crew_for_task` actually reads
/// `CONSTELLATION_DEFAULT_PROVIDER`, and the environment tier stays subordinate
/// to a configured `[workflow].default_crew`. Because a workspace default beats
/// the env tier, a leaked value cannot affect concurrent tests.
#[test]
fn resolve_crew_for_task_reads_env_default_but_workspace_crew_wins() {
    let _lock = env_lock();
    let saved = std::env::var(CONSTELLATION_DEFAULT_PROVIDER_ENV).ok();
    // SAFETY: serialized by env_lock(); restored before we assert or return.
    unsafe { std::env::set_var(CONSTELLATION_DEFAULT_PROVIDER_ENV, "claude") };

    let (_root, runtime) = runtime_with_named_crews();
    let resolved = runtime.resolve_crew_for_task(None, None);

    // Restore the prior environment before asserting so a failure never leaks.
    // SAFETY: same serialization lock still held.
    unsafe {
        match saved {
            Some(value) => std::env::set_var(CONSTELLATION_DEFAULT_PROVIDER_ENV, value),
            None => std::env::remove_var(CONSTELLATION_DEFAULT_PROVIDER_ENV),
        }
    }

    let crew = resolved.expect("resolve crew");
    // `[workflow].default_crew = "primary"` (workspace tier) beats the env tier.
    assert_eq!(crew.name, "primary");
}
