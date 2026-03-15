use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;

fn orbit_in(dir: &std::path::Path) -> Command {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    cmd.current_dir(dir);
    cmd.env("HOME", dir);
    cmd.env("USERPROFILE", dir);
    cmd
}

#[cfg(unix)]
fn create_dir_symlink(src: &std::path::Path, dst: &std::path::Path) {
    std::os::unix::fs::symlink(src, dst).expect("create symlink");
}

#[cfg(windows)]
fn create_dir_symlink(src: &std::path::Path, dst: &std::path::Path) {
    std::os::windows::fs::symlink_dir(src, dst).expect("create symlink");
}

fn assert_default_skill_links(base_root: &std::path::Path) {
    for skills_link_root in [
        base_root.join(".agents").join("skills"),
        base_root.join(".claude").join("skills"),
    ] {
        let root_meta =
            std::fs::symlink_metadata(&skills_link_root).expect("skills link dir metadata");
        assert!(root_meta.file_type().is_dir());
        for skill_id in [
            "orbit-create-task",
            "orbit-approve-task",
            "orbit-assess-codebase",
            "orbit-execute-change-request",
            "orbit-maintain-system",
            "orbit-operations-management",
            "orbit-manage-tasks",
            "orbit-skills",
            "orbit-track-issues",
        ] {
            let link_path = skills_link_root.join(skill_id);
            let link_meta = std::fs::symlink_metadata(&link_path).expect("skill symlink metadata");
            assert!(link_meta.file_type().is_symlink());
        }
    }
}

fn assert_default_named_jobs_visible_and_enabled(base_root: &std::path::Path) {
    let list_output = orbit_in(base_root)
        .args(["job", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let list: serde_json::Value = serde_json::from_slice(&list_output).expect("job list json");
    let jobs = list.as_array().expect("jobs array");

    for job_id in [
        "job_perform_maintenance",
        "job_oversee_orbit_operations",
        "job_review_tasks",
        "job_task_pipeline",
    ] {
        let job = jobs
            .iter()
            .find(|job| job["job_id"] == job_id)
            .unwrap_or_else(|| panic!("missing default job in list: {job_id}"));
        assert_eq!(job["state"], "enabled");
    }
}

fn assert_default_named_activities_visible(base_root: &std::path::Path) {
    let list_output = orbit_in(base_root)
        .args(["activity", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let list: serde_json::Value = serde_json::from_slice(&list_output).expect("activity list json");
    let activities = list.as_array().expect("activities array");

    for activity_id in [
        "checkout_branch",
        "create_branch",
        "dispatch_task",
        "implement_change",
        "open_pr",
        "oversee_orbit_operations",
        "perform_maintenance",
        "review_pr",
        "review_tasks",
        "run_tests",
    ] {
        activities
            .iter()
            .find(|activity| activity["id"] == activity_id)
            .unwrap_or_else(|| panic!("missing default activity in list: {activity_id}"));
    }
}

fn rewrite_file(path: &std::path::Path, replacements: &[(&str, &str)]) {
    let mut raw = std::fs::read_to_string(path).expect("read file");
    for (old, new) in replacements {
        raw = raw.replace(old, new);
    }
    std::fs::write(path, raw).expect("write file");
}

fn rename_seeded_file_to_legacy_name(
    dir: &std::path::Path,
    current_id: &str,
    legacy_id: &str,
    replacements: &[(&str, &str)],
) {
    let current_path = dir.join(format!("{current_id}.yaml"));
    let legacy_path = dir.join(format!("{legacy_id}.yaml"));
    std::fs::rename(&current_path, &legacy_path).expect("rename legacy file");
    rewrite_file(&legacy_path, replacements);
}

#[test]
fn init_creates_default_identities_under_cwd_orbit() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home = tempfile::tempdir().expect("home");

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("identities: root="))
        .stdout(predicate::str::contains("refreshed="))
        .stdout(predicate::str::contains("skills: root="))
        .stdout(predicate::str::contains("config: path="));

    let identity_root = workspace.path().join(".orbit").join("identities");
    assert!(identity_root.join("linus.yaml").exists());
    assert!(identity_root.join("lamport.yaml").exists());
    assert!(identity_root.join("prii.yaml").exists());
    assert!(identity_root.join("steve.yaml").exists());
    assert!(!identity_root.join("grace.yaml").exists());
    assert!(!identity_root.join("john.yaml").exists());
    assert!(!identity_root.join("kent.yaml").exists());
    assert!(!identity_root.join("rob.yaml").exists());

    let skills_root = workspace.path().join(".orbit").join("skills");
    assert!(
        skills_root
            .join("orbit-create-task")
            .join("SKILL.md")
            .exists()
    );
    assert!(
        skills_root
            .join("orbit-approve-task")
            .join("SKILL.md")
            .exists()
    );
    assert!(
        skills_root
            .join("orbit-assess-codebase")
            .join("SKILL.md")
            .exists()
    );
    assert!(
        skills_root
            .join("orbit-execute-change-request")
            .join("SKILL.md")
            .exists()
    );
    assert!(
        skills_root
            .join("orbit-maintain-system")
            .join("SKILL.md")
            .exists()
    );
    assert!(
        skills_root
            .join("orbit-operations-management")
            .join("SKILL.md")
            .exists()
    );
    assert!(
        skills_root
            .join("orbit-manage-tasks")
            .join("SKILL.md")
            .exists()
    );
    assert!(skills_root.join("orbit-skills").join("SKILL.md").exists());
    assert!(
        skills_root
            .join("orbit-track-issues")
            .join("SKILL.md")
            .exists()
    );

    let config_path = workspace.path().join(".orbit").join("config.toml");
    assert!(config_path.exists());
    let config_raw = std::fs::read_to_string(config_path).expect("read config");
    assert!(config_raw.contains("[execution.env]"));
    assert!(config_raw.contains("[execution.codex]"));
    assert!(config_raw.contains("[task.approval]"));
    assert!(!config_raw.contains("[watch]"));

    assert_default_skill_links(workspace.path());
    assert_default_named_activities_visible(workspace.path());
    assert_default_named_jobs_visible_and_enabled(workspace.path());
}

#[test]
fn init_refreshes_full_bundled_activity_and_job_set() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home = tempfile::tempdir().expect("home");

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("default_activities_refreshed=10"))
        .stdout(predicate::str::contains("default_jobs_refreshed=4"));

    let activities_dir = workspace
        .path()
        .join(".orbit")
        .join("activities")
        .join("active");
    for activity_id in [
        "checkout_branch",
        "create_branch",
        "dispatch_task",
        "implement_change",
        "open_pr",
        "oversee_orbit_operations",
        "perform_maintenance",
        "review_pr",
        "review_tasks",
        "run_tests",
    ] {
        assert!(
            activities_dir.join(format!("{activity_id}.yaml")).exists(),
            "missing activity file: {activity_id}"
        );
    }

    let jobs_dir = workspace.path().join(".orbit").join("jobs").join("jobs");
    for job_id in [
        "job_oversee_orbit_operations",
        "job_perform_maintenance",
        "job_review_tasks",
        "job_task_pipeline",
    ] {
        assert!(
            jobs_dir.join(format!("{job_id}.yaml")).exists(),
            "missing job file: {job_id}"
        );
    }

    assert_default_named_activities_visible(workspace.path());
    assert_default_named_jobs_visible_and_enabled(workspace.path());
}

#[test]
fn init_is_idempotent_for_existing_identity_files() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home = tempfile::tempdir().expect("home");

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("identities: root="))
        .stdout(predicate::str::contains("refreshed=4"))
        .stdout(predicate::str::contains("skills: root="))
        .stdout(predicate::str::contains("refreshed=9"));

    // Second init also refreshes all defaults (overwrite in place).
    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("identities: root="))
        .stdout(predicate::str::contains("refreshed=4"))
        .stdout(predicate::str::contains("skills: root="))
        .stdout(predicate::str::contains("refreshed=9"));
}

#[test]
fn explicit_init_refreshes_builtin_activities_and_jobs_but_implicit_bootstrap_does_not() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home = tempfile::tempdir().expect("home");

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success();

    let activity_path = workspace
        .path()
        .join(".orbit")
        .join("activities")
        .join("active")
        .join("dispatch_task.yaml");
    let job_path = workspace
        .path()
        .join(".orbit")
        .join("jobs")
        .join("jobs")
        .join("job_task_pipeline.yaml");

    rewrite_file(
        &activity_path,
        &[("Pick the single best task", "TAMPERED ACTIVITY")],
    );
    rewrite_file(&job_path, &[("dispatch_task", "tampered_dispatch_task")]);

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["task", "list", "--json"])
        .assert()
        .success();

    let activity_raw = std::fs::read_to_string(&activity_path).expect("read activity");
    assert!(activity_raw.contains("TAMPERED ACTIVITY"));
    let job_raw = std::fs::read_to_string(&job_path).expect("read job");
    assert!(job_raw.contains("tampered_dispatch_task"));

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("default_activities_refreshed=10"))
        .stdout(predicate::str::contains("default_jobs_refreshed=4"));

    let refreshed_activity_raw = std::fs::read_to_string(&activity_path).expect("read activity");
    assert!(!refreshed_activity_raw.contains("TAMPERED ACTIVITY"));
    assert!(refreshed_activity_raw.contains("Pick the single best task"));

    let refreshed_job_raw = std::fs::read_to_string(&job_path).expect("read job");
    assert!(!refreshed_job_raw.contains("tampered_dispatch_task"));
    assert!(refreshed_job_raw.contains("dispatch_task"));
}

#[test]
fn init_migrates_root_skills_symlink_to_per_skill_links() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home = tempfile::tempdir().expect("home");

    let orbit_skills = workspace.path().join(".orbit").join("skills");
    std::fs::create_dir_all(&orbit_skills).expect("create orbit skills");
    for skill_parent in [".agents", ".claude"] {
        let skill_dir = workspace.path().join(skill_parent);
        std::fs::create_dir_all(&skill_dir).expect("create skill dir");
        create_dir_symlink(&orbit_skills, &skill_dir.join("skills"));
    }

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success();

    for skills_link_root in [
        workspace.path().join(".agents").join("skills"),
        workspace.path().join(".claude").join("skills"),
    ] {
        let root_meta = std::fs::symlink_metadata(&skills_link_root).expect("skills metadata");
        assert!(root_meta.file_type().is_dir());
        assert!(!root_meta.file_type().is_symlink());
        let skill_link_meta =
            std::fs::symlink_metadata(skills_link_root.join("orbit-approve-task"))
                .expect("orbit-approve-task link metadata");
        assert!(skill_link_meta.file_type().is_symlink());
    }
}

#[test]
fn init_repairs_broken_per_skill_symlink_targets() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home = tempfile::tempdir().expect("home");

    let broken_target = workspace
        .path()
        .join(".orbit")
        .join("skills")
        .join("does-not-exist");
    for skills_link_root in [
        workspace.path().join(".agents").join("skills"),
        workspace.path().join(".claude").join("skills"),
    ] {
        std::fs::create_dir_all(&skills_link_root).expect("create skills link root");
        create_dir_symlink(&broken_target, &skills_link_root.join("orbit-approve-task"));
    }

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success();

    let expected_target = workspace
        .path()
        .join(".orbit")
        .join("skills")
        .join("orbit-approve-task")
        .canonicalize()
        .expect("canonical expected target");
    for repaired_link in [
        workspace
            .path()
            .join(".agents")
            .join("skills")
            .join("orbit-approve-task"),
        workspace
            .path()
            .join(".claude")
            .join("skills")
            .join("orbit-approve-task"),
    ] {
        let repaired_link_meta =
            std::fs::symlink_metadata(&repaired_link).expect("repaired metadata");
        assert!(repaired_link_meta.file_type().is_symlink());
        assert!(repaired_link.exists());
        let actual_target = repaired_link
            .canonicalize()
            .expect("canonical repaired target");
        assert_eq!(actual_target, expected_target);
    }
}

#[test]
fn init_force_resets_cwd_orbit_to_defaults() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home = tempfile::tempdir().expect("home");

    let orbit_root = workspace.path().join(".orbit");
    std::fs::create_dir_all(orbit_root.join("skills").join("orbit-approve-task"))
        .expect("create legacy skills");
    std::fs::write(
        orbit_root
            .join("skills")
            .join("orbit-approve-task")
            .join("SKILL.md"),
        "LEGACY CONTENT",
    )
    .expect("write legacy skill");
    std::fs::write(
        orbit_root.join("config.toml"),
        "custom_config = true\n# legacy marker",
    )
    .expect("write legacy config");
    std::fs::create_dir_all(orbit_root.join("junk")).expect("create junk dir");
    std::fs::write(orbit_root.join("junk").join("stale.txt"), "stale").expect("write stale file");

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init", "--force"])
        .assert()
        .success();

    let config_path = orbit_root.join("config.toml");
    let config_raw = std::fs::read_to_string(config_path).expect("read config");
    assert!(config_raw.contains("[execution.env]"));
    assert!(!config_raw.contains("[watch]"));
    assert!(!config_raw.contains("legacy marker"));

    let skill_raw = std::fs::read_to_string(
        orbit_root
            .join("skills")
            .join("orbit-approve-task")
            .join("SKILL.md"),
    )
    .expect("read seeded skill");
    assert!(skill_raw.contains("name: orbit-approve-task"));
    assert!(!skill_raw.contains("LEGACY CONTENT"));

    assert!(!orbit_root.join("junk").exists());
}

#[test]
fn init_uses_repo_local_layout_when_inside_git_repository() {
    let repo = tempfile::tempdir().expect("repo");
    let home = tempfile::tempdir().expect("home");
    let repo_canonical = repo.path().canonicalize().expect("canonical repo path");

    std::fs::create_dir_all(repo.path().join(".git")).expect("create git marker");
    let nested = repo.path().join("nested").join("workdir");
    std::fs::create_dir_all(&nested).expect("create nested workdir");
    let repo_orbit = repo.path().join(".orbit");

    orbit_in(&nested)
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "identities: root={}",
            repo_canonical.join(".orbit").join("identities").display()
        )))
        .stdout(predicate::str::contains(format!(
            "skills: root={}",
            repo_canonical.join(".orbit").join("skills").display()
        )));

    assert!(repo_orbit.join("identities").join("prii.yaml").exists());
    assert!(
        repo_orbit
            .join("skills")
            .join("orbit-approve-task")
            .join("SKILL.md")
            .exists()
    );

    assert_default_skill_links(repo.path());

    let config_raw =
        std::fs::read_to_string(repo_orbit.join("config.toml")).expect("read repo config");
    assert!(config_raw.contains("path = \"skills\""));
    assert!(config_raw.contains("path = \"orbit.db\""));

    assert_default_named_jobs_visible_and_enabled(repo.path());
}

#[test]
fn init_refreshes_modified_defaults_without_destroying_tasks() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home = tempfile::tempdir().expect("home");

    // First init to seed everything.
    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success();

    let orbit_root = workspace.path().join(".orbit");

    // Tamper with a default identity file.
    let identity_path = orbit_root.join("identities").join("prii.yaml");
    std::fs::write(&identity_path, "TAMPERED IDENTITY").expect("tamper identity");

    // Tamper with a default skill file.
    let skill_path = orbit_root
        .join("skills")
        .join("orbit-approve-task")
        .join("SKILL.md");
    std::fs::write(&skill_path, "TAMPERED SKILL").expect("tamper skill");

    // Create a fake task artifact that must survive.
    let task_dir = orbit_root.join("tasks").join("backlog").join("T-fake-task");
    std::fs::create_dir_all(&task_dir).expect("create task dir");
    std::fs::write(task_dir.join("task.yaml"), "id: T-fake-task\n").expect("write task");

    // Re-run plain init (no --force).
    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("refreshed="));

    // Identity should be restored to default.
    let identity_raw = std::fs::read_to_string(&identity_path).expect("read identity");
    assert!(!identity_raw.contains("TAMPERED"));
    assert!(identity_raw.contains("display_name:"));

    // Skill should be restored to default.
    let skill_raw = std::fs::read_to_string(&skill_path).expect("read skill");
    assert!(!skill_raw.contains("TAMPERED"));
    assert!(skill_raw.contains("name: orbit-approve-task"));

    // Task artifact must still exist.
    assert!(task_dir.join("task.yaml").exists());
}

#[test]
fn init_migrates_legacy_builtin_kebab_case_names_to_snake_case() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home = tempfile::tempdir().expect("home");

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success();

    let orbit_root = workspace.path().join(".orbit");
    let activities_dir = orbit_root.join("activities").join("active");
    let jobs_dir = orbit_root.join("jobs").join("jobs");

    rename_seeded_file_to_legacy_name(
        &activities_dir,
        "review_tasks",
        "approve-task-leader",
        &[("review_tasks", "approve-task-leader")],
    );
    rename_seeded_file_to_legacy_name(
        &activities_dir,
        "dispatch_task",
        "triage-and-dispatch-task",
        &[("dispatch_task", "triage-and-dispatch-task")],
    );
    rename_seeded_file_to_legacy_name(
        &jobs_dir,
        "job_review_tasks",
        "job-approve-task-leader",
        &[
            ("job_review_tasks", "job-approve-task-leader"),
            ("review_tasks", "approve-task-leader"),
        ],
    );

    let legacy_run_dir = orbit_root
        .join("jobs")
        .join("runs")
        .join("job-approve-task-leader")
        .join("jrun-20260315-010101");
    std::fs::create_dir_all(legacy_run_dir.join("steps")).expect("create legacy run bundle");
    std::fs::write(
        legacy_run_dir.join("jrun.yaml"),
        r#"schemaVersion: 1
run:
  run_id: jrun-20260315-010101
  job_id: job-approve-task-leader
  attempt: 1
  state: success
  scheduled_at: 2026-03-15T01:01:01Z
  started_at: 2026-03-15T01:01:02Z
  finished_at: 2026-03-15T01:01:03Z
  duration_ms: 1000
  created_at: 2026-03-15T01:01:01Z
"#,
    )
    .expect("write legacy jrun");
    std::fs::write(
        legacy_run_dir
            .join("steps")
            .join("01-approve-task-leader.yaml"),
        r#"schemaVersion: 1
step:
  step_index: 0
  target_type: activity
  target_id: approve-task-leader
  started_at: 2026-03-15T01:01:02Z
  finished_at: 2026-03-15T01:01:03Z
  duration_ms: 1000
  exit_code: 0
  agent_response_json:
    schemaVersion: 1
    status: success
    result: {}
    error: null
    durationMs: 1
  state: success
  error_code: null
  error_message: null
"#,
    )
    .expect("write legacy step");

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success();

    assert!(activities_dir.join("review_tasks.yaml").exists());
    assert!(!activities_dir.join("approve-task-leader.yaml").exists());
    assert!(activities_dir.join("dispatch_task.yaml").exists());
    assert!(
        !activities_dir
            .join("triage-and-dispatch-task.yaml")
            .exists()
    );

    assert!(jobs_dir.join("job_review_tasks.yaml").exists());
    assert!(!jobs_dir.join("job-approve-task-leader.yaml").exists());

    let migrated_run_dir = orbit_root
        .join("jobs")
        .join("runs")
        .join("job_review_tasks")
        .join("jrun-20260315-010101");
    assert!(migrated_run_dir.exists());
    assert!(
        !orbit_root
            .join("jobs")
            .join("runs")
            .join("job-approve-task-leader")
            .exists()
    );
    assert!(
        migrated_run_dir
            .join("steps")
            .join("01-review_tasks.yaml")
            .exists()
    );

    let migrated_jrun =
        std::fs::read_to_string(migrated_run_dir.join("jrun.yaml")).expect("read migrated jrun");
    assert!(migrated_jrun.contains("job_review_tasks"));
    assert!(!migrated_jrun.contains("job-approve-task-leader"));

    let migrated_step =
        std::fs::read_to_string(migrated_run_dir.join("steps").join("01-review_tasks.yaml"))
            .expect("read migrated step");
    assert!(migrated_step.contains("review_tasks"));
    assert!(!migrated_step.contains("approve-task-leader"));

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["activity", "show", "review_tasks", "--json"])
        .assert()
        .success();
    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["activity", "show", "dispatch_task", "--json"])
        .assert()
        .success();

    let history_output = orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["job", "history", "job_review_tasks", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let history: Value = serde_json::from_slice(&history_output).expect("history json");
    let runs = history.as_array().expect("history array");
    assert!(runs.iter().any(|run| run["job_id"] == "job_review_tasks"));
}
