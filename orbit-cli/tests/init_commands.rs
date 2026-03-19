use assert_cmd::Command;
use predicates::prelude::*;

fn orbit_in(dir: &std::path::Path) -> Command {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    cmd.current_dir(dir);
    cmd.env("HOME", dir);
    cmd.env("USERPROFILE", dir);
    cmd.env("ORBIT_ROOT", dir.join(".orbit"));
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
            "orbit",
            "orbit-create-task",
            "orbit-approve-task",
            "orbit-execute-change-request",
            "orbit-maintain-system",
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
        "commit_changes",
        "create_branch",
        "start_task",
        "update_task",
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

#[test]
fn init_creates_default_runtime_assets_under_cwd_orbit() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home = tempfile::tempdir().expect("home");

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("skills: root="))
        .stdout(predicate::str::contains("config: path="));

    let skills_root = workspace.path().join(".orbit").join("skills");
    assert!(skills_root.join("orbit").join("SKILL.md").exists());
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
        .stdout(predicate::str::contains("default_activities_refreshed=13"))
        .stdout(predicate::str::contains("default_jobs_refreshed=4"));

    let activities_dir = workspace
        .path()
        .join(".orbit")
        .join("activities")
        .join("active");
    for activity_id in [
        "checkout_branch",
        "commit_changes",
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
fn init_is_idempotent_for_existing_defaults() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home = tempfile::tempdir().expect("home");

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("skills: root="))
        .stdout(predicate::str::contains("refreshed=6"));

    // Second init also refreshes all defaults (overwrite in place).
    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("skills: root="))
        .stdout(predicate::str::contains("refreshed=6"));
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
        .stdout(predicate::str::contains("default_activities_refreshed=13"))
        .stdout(predicate::str::contains("default_jobs_refreshed=4"));

    let refreshed_activity_raw = std::fs::read_to_string(&activity_path).expect("read activity");
    assert!(!refreshed_activity_raw.contains("TAMPERED ACTIVITY"));
    assert!(refreshed_activity_raw.contains("Pick the single best task"));

    let refreshed_job_raw = std::fs::read_to_string(&job_path).expect("read job");
    assert!(!refreshed_job_raw.contains("tampered_dispatch_task"));
    assert!(refreshed_job_raw.contains("dispatch_task"));
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
fn init_uses_explicit_orbit_root_when_invoked_inside_git_repository() {
    let repo = tempfile::tempdir().expect("repo");
    let home = tempfile::tempdir().expect("home");
    std::fs::create_dir_all(repo.path().join(".git")).expect("create git marker");
    let nested = repo.path().join("nested").join("workdir");
    std::fs::create_dir_all(&nested).expect("create nested workdir");
    let repo_orbit = nested.join(".orbit");

    orbit_in(&nested)
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "skills: root={}",
            nested.join(".orbit").join("skills").display()
        )));

    assert!(
        nested
            .join(".orbit")
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

    // Skill should be restored to default.
    let skill_raw = std::fs::read_to_string(&skill_path).expect("read skill");
    assert!(!skill_raw.contains("TAMPERED"));
    assert!(skill_raw.contains("name: orbit-approve-task"));

    // Task artifact must still exist.
    assert!(task_dir.join("task.yaml").exists());
}
