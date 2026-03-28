//! Integration tests for scoping, identity attribution, and path resolution invariants.
//!
//! These tests verify production-discovered bugs do not regress:
//! - Tasks are workspace-scoped (never leak to global root)
//! - System-generated failure tasks have correct identity attribution
//! - Friction bounty is not incremented for system-generated tasks
//! - WorkspacePaths derives all sub-directories from orbit_dir
//! - Path resolution handles worktree contexts correctly

use std::fs;
use std::path::PathBuf;

use orbit_core::OrbitRuntime;
use orbit_core::command::task::TaskAddParams;
use orbit_types::{ActorIdentity, TaskType, WorkspacePaths};
use tempfile::tempdir;

// ---------------------------------------------------------------------------
// Helper: create a runtime with scoring enabled
// ---------------------------------------------------------------------------

fn runtime_with_scoring(data_root: &std::path::Path) -> OrbitRuntime {
    // Seed the directory and write a config that enables scoring
    fs::create_dir_all(data_root).expect("create data_root");
    let config_path = data_root.join("config.toml");
    fs::write(
        &config_path,
        "[scoring]\nenabled = true\n\n[task.approval]\nrequired_for_agent = false\n",
    )
    .expect("write config.toml");
    // Create scoreboard dir so friction_bounty writes succeed
    let scoreboard_dir = data_root.join("scoreboard");
    fs::create_dir_all(&scoreboard_dir).expect("create scoreboard dir");
    fs::write(scoreboard_dir.join("friction_bounty.json"), "{}\n")
        .expect("seed friction_bounty.json");
    OrbitRuntime::from_data_root(data_root).expect("build runtime with scoring")
}

// ---------------------------------------------------------------------------
// 1. Global init does NOT create workspace-scoped dirs
// ---------------------------------------------------------------------------

#[test]
fn init_global_only_skips_workspace_scoped_artifacts() {
    let dir = tempdir().expect("tempdir");
    let global_root = dir.path().join("global_orbit");

    let runtime = OrbitRuntime::from_data_root(&global_root).expect("runtime");
    runtime
        .init_workspace_with_options(orbit_core::command::init::InitOptions {
            force: false,
            refresh_defaults: true,
            global_only: true,
        })
        .expect("init global_only");

    // tasks/ must NOT exist at global scope (WorkspaceOnly)
    assert!(
        !global_root.join("tasks").exists(),
        "tasks/ must not be created at global scope"
    );

    // scoreboard/ must NOT exist at global scope (workspace-scoped)
    assert!(
        !global_root.join("scoreboard").exists(),
        "scoreboard/ must not be created at global scope"
    );

    // runs/ must NOT exist at global scope (WorkspaceOnly)
    assert!(
        !global_root.join("runs").exists(),
        "runs/ must not be created at global scope"
    );
}

// ---------------------------------------------------------------------------
// 2. Workspace init creates workspace-scoped artifacts
// ---------------------------------------------------------------------------

#[test]
fn workspace_init_creates_tasks_and_scoreboard() {
    let dir = tempdir().expect("tempdir");
    let workspace_root = dir.path().join("workspace_orbit");

    let runtime = OrbitRuntime::from_data_root(&workspace_root).expect("runtime");
    runtime
        .init_workspace_with_options(orbit_core::command::init::InitOptions {
            force: false,
            refresh_defaults: true,
            global_only: false,
        })
        .expect("init workspace");

    // tasks/ dir should NOT be created by init_workspace (that's ensure_orbit_root_initialized's job),
    // but the workspace itself should exist.
    assert!(workspace_root.exists(), "workspace root should be created");

    // When scoring is enabled, scoreboard should be seeded at workspace scope.
    // Default config has scoring=false, so scoreboard is not created.
    // Test with scoring enabled:
    let scored_root = dir.path().join("scored_orbit");
    let scored_runtime = runtime_with_scoring(&scored_root);
    scored_runtime
        .init_workspace_with_options(orbit_core::command::init::InitOptions {
            force: false,
            refresh_defaults: true,
            global_only: false,
        })
        .expect("init workspace with scoring");

    assert!(
        scored_root.join("scoreboard").exists(),
        "scoreboard/ should exist at workspace scope when scoring enabled"
    );
    assert!(
        scored_root.join("scoreboard").join("pr.json").exists(),
        "pr.json scoreboard template should be seeded"
    );
    assert!(
        scored_root
            .join("scoreboard")
            .join("friction_bounty.json")
            .exists(),
        "friction_bounty.json scoreboard template should be seeded"
    );
}

// ---------------------------------------------------------------------------
// 3. Failure task with no agent/model has system identity
// ---------------------------------------------------------------------------

#[test]
fn task_created_without_agent_has_system_identity() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");

    // Simulate what maybe_create_failure_task does: add_task_with_identity with None, None
    let task = runtime
        .add_task_with_identity(
            TaskAddParams {
                title: "Job failure: test_job [timeout]".to_string(),
                description: "Test failure task".to_string(),
                plan: "1. Investigate".to_string(),
                task_type: TaskType::Friction,
                ..Default::default()
            },
            None, // no agent
            None, // no model
        )
        .expect("create failure task");

    // actor_identity must be System variant (not Agent) when no agent/model provided.
    // This is the authoritative identity field used for attribution decisions.
    assert_eq!(
        task.actor_identity,
        ActorIdentity::System,
        "system-generated failure task must have ActorIdentity::System"
    );

    // Verify the identity is recognized as system (not agent)
    assert!(
        task.actor_identity.is_system(),
        "system-generated task identity must report is_system()=true"
    );
    assert!(
        !task.actor_identity.is_agent(),
        "system-generated task identity must report is_agent()=false"
    );
}

#[test]
fn task_created_with_agent_has_agent_identity() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");

    let task = runtime
        .add_task_with_identity(
            TaskAddParams {
                title: "Agent-created task".to_string(),
                description: "Test agent task".to_string(),
                plan: "1. Do the thing".to_string(),
                task_type: TaskType::Friction,
                ..Default::default()
            },
            Some("claude".to_string()),
            Some("opus-4.6".to_string()),
        )
        .expect("create agent task");

    assert_eq!(
        task.created_by.as_deref(),
        Some("claude / opus-4.6"),
        "agent-created task must have created_by='agent / model'"
    );
    assert_eq!(
        task.actor_identity,
        ActorIdentity::agent("claude", "opus-4.6"),
        "agent-created task must have ActorIdentity::Agent"
    );
}

// ---------------------------------------------------------------------------
// 4. Friction bounty NOT incremented for system-generated friction tasks
// ---------------------------------------------------------------------------

#[test]
fn friction_bounty_not_incremented_for_system_friction_task() {
    let dir = tempdir().expect("tempdir");
    let runtime = runtime_with_scoring(dir.path());

    let scoreboard_path = dir.path().join("scoreboard").join("friction_bounty.json");
    let before: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&scoreboard_path).unwrap()).unwrap();

    // Create a friction task with NO agent/model (system-generated)
    let _task = runtime
        .add_task_with_identity(
            TaskAddParams {
                title: "System friction task".to_string(),
                description: "Auto-generated".to_string(),
                plan: "1. Fix".to_string(),
                task_type: TaskType::Friction,
                ..Default::default()
            },
            None,
            None,
        )
        .expect("create system friction task");

    let after: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&scoreboard_path).unwrap()).unwrap();

    assert_eq!(
        before, after,
        "friction bounty must NOT be incremented for system-generated friction tasks"
    );
}

#[test]
fn friction_bounty_incremented_for_agent_friction_task() {
    let dir = tempdir().expect("tempdir");
    let runtime = runtime_with_scoring(dir.path());

    let scoreboard_path = dir.path().join("scoreboard").join("friction_bounty.json");

    // Create a friction task WITH agent/model
    let _task = runtime
        .add_task_with_identity(
            TaskAddParams {
                title: "Agent friction task".to_string(),
                description: "Agent-reported".to_string(),
                plan: "1. Fix".to_string(),
                task_type: TaskType::Friction,
                ..Default::default()
            },
            Some("claude".to_string()),
            Some("opus-4.6".to_string()),
        )
        .expect("create agent friction task");

    let after: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&scoreboard_path).unwrap()).unwrap();

    assert_eq!(
        after["issues-reported"]["claude"]["opus-4.6"], 1,
        "friction bounty MUST be incremented for agent-created friction tasks"
    );
}

// ---------------------------------------------------------------------------
// 5. WorkspacePaths resolves correctly
// ---------------------------------------------------------------------------

#[test]
fn workspace_paths_derives_all_subdirs_from_orbit_dir() {
    let repo_root = PathBuf::from("/repo");
    let orbit_dir = PathBuf::from("/repo/.orbit");
    let global_dir = PathBuf::from("/home/user/.orbit");

    let paths = WorkspacePaths::new(repo_root.clone(), orbit_dir.clone(), global_dir.clone());

    assert_eq!(paths.repo_root, repo_root);
    assert_eq!(paths.orbit_dir, orbit_dir);
    assert_eq!(paths.global_dir, global_dir);
    assert_eq!(paths.tasks_dir, orbit_dir.join("tasks"));
    assert_eq!(paths.activities_dir, orbit_dir.join("activities"));
    assert_eq!(paths.jobs_dir, orbit_dir.join("jobs"));
    assert_eq!(paths.runs_dir, orbit_dir.join("runs"));
    assert_eq!(paths.skills_dir, orbit_dir.join("skills"));
    assert_eq!(paths.scoreboard_dir, orbit_dir.join("scoreboard"));
    assert_eq!(paths.diagnostics_dir, orbit_dir.join("diagnostics"));
}

#[test]
fn workspace_paths_global_dir_independent_of_orbit_dir() {
    let paths = WorkspacePaths::new(
        PathBuf::from("/worktree/task-123"),
        PathBuf::from("/worktree/task-123/.orbit"),
        PathBuf::from("/main-repo/.orbit"),
    );

    // Workspace-scoped paths derive from orbit_dir, not global_dir
    assert_eq!(
        paths.tasks_dir,
        PathBuf::from("/worktree/task-123/.orbit/tasks"),
        "tasks must derive from workspace orbit_dir"
    );
    assert_eq!(
        paths.scoreboard_dir,
        PathBuf::from("/worktree/task-123/.orbit/scoreboard"),
        "scoreboard must derive from workspace orbit_dir"
    );

    // Global dir is stored independently
    assert_eq!(
        paths.global_dir,
        PathBuf::from("/main-repo/.orbit"),
        "global_dir must be the main repo's .orbit"
    );
}

// ---------------------------------------------------------------------------
// 6. Two-root runtime: tasks go to workspace, not global
// ---------------------------------------------------------------------------

#[test]
fn two_root_runtime_writes_tasks_to_workspace_not_global() {
    let dir = tempdir().expect("tempdir");
    let global_root = dir.path().join("global");
    let workspace_root = dir.path().join("workspace");

    // Bootstrap both roots
    fs::create_dir_all(&global_root).expect("mkdir global");
    fs::create_dir_all(&workspace_root).expect("mkdir workspace");

    // Create minimal configs
    fs::write(
        global_root.join("config.toml"),
        "[task.approval]\nrequired_for_agent = false\n",
    )
    .expect("write global config");

    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("two-root runtime");

    let task = runtime
        .add_task(TaskAddParams {
            title: "Workspace-scoped task".to_string(),
            description: "Must land in workspace".to_string(),
            plan: "1. Verify".to_string(),
            ..Default::default()
        })
        .expect("add task");

    // Task file must exist under workspace root, not global
    let workspace_task_path = workspace_root
        .join("tasks")
        .join("backlog")
        .join(&task.id.to_string())
        .join("task.yaml");
    assert!(
        workspace_task_path.exists(),
        "task must be written to workspace tasks dir: {}",
        workspace_task_path.display()
    );

    // Global root must NOT have task artifacts
    let global_tasks = global_root.join("tasks");
    if global_tasks.exists() {
        let entries: Vec<_> = fs::read_dir(&global_tasks)
            .expect("read global tasks dir")
            .filter_map(|e| e.ok())
            .filter(|e| {
                // Ignore empty status subdirectories that ensure_layout creates
                e.path().is_dir()
                    && fs::read_dir(e.path())
                        .map(|mut d| d.next().is_some())
                        .unwrap_or(false)
            })
            .collect();
        assert!(
            entries.is_empty(),
            "global tasks dir must not contain task files, found: {:?}",
            entries.iter().map(|e| e.path()).collect::<Vec<_>>()
        );
    }
}
