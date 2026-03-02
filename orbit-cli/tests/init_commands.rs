use assert_cmd::Command;
use predicates::prelude::*;

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

#[test]
fn init_creates_default_identities_under_home_orbit() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home = tempfile::tempdir().expect("home");

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("identities: root="))
        .stdout(predicate::str::contains("skills: root="))
        .stdout(predicate::str::contains("config: path="));

    let identity_root = home.path().join(".orbit").join("identities");
    assert!(identity_root.join("linus.yaml").exists());
    assert!(identity_root.join("john.yaml").exists());
    assert!(identity_root.join("kent.yaml").exists());
    assert!(identity_root.join("rob.yaml").exists());
    assert!(identity_root.join("grace.yaml").exists());
    assert!(identity_root.join("steve.yaml").exists());

    let skills_root = home.path().join(".orbit").join("skills");
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
            .join("orbit-manage-tasks")
            .join("SKILL.md")
            .exists()
    );
    assert!(
        skills_root
            .join("orbit-track-issues")
            .join("SKILL.md")
            .exists()
    );

    let config_path = home.path().join(".orbit").join("config.toml");
    assert!(config_path.exists());
    let config_raw = std::fs::read_to_string(config_path).expect("read config");
    assert!(config_raw.contains("[execution.env]"));
    assert!(config_raw.contains("[task.approval]"));

    let skills_link_root = home.path().join(".agents").join("skills");
    let root_meta = std::fs::symlink_metadata(&skills_link_root).expect("skills link dir metadata");
    assert!(root_meta.file_type().is_dir());
    for skill_id in [
        "orbit-approve-task",
        "orbit-assess-codebase",
        "orbit-execute-change-request",
        "orbit-maintain-system",
        "orbit-manage-tasks",
        "orbit-track-issues",
    ] {
        let link_path = skills_link_root.join(skill_id);
        let link_meta = std::fs::symlink_metadata(&link_path).expect("skill symlink metadata");
        assert!(link_meta.file_type().is_symlink());
    }
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
        .stdout(predicate::str::contains("created=0"))
        .stdout(predicate::str::contains("skills: root="))
        .stdout(predicate::str::contains("created=0"))
        .stdout(predicate::str::contains("created=false"));

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("identities: root="))
        .stdout(predicate::str::contains("created=0"))
        .stdout(predicate::str::contains("skills: root="))
        .stdout(predicate::str::contains("created=0"))
        .stdout(predicate::str::contains("created=false"));
}

#[test]
fn init_migrates_root_skills_symlink_to_per_skill_links() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home = tempfile::tempdir().expect("home");

    let orbit_skills = home.path().join(".orbit").join("skills");
    std::fs::create_dir_all(&orbit_skills).expect("create orbit skills");
    let agents_dir = home.path().join(".agents");
    std::fs::create_dir_all(&agents_dir).expect("create agents dir");
    create_dir_symlink(&orbit_skills, &agents_dir.join("skills"));

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success();

    let skills_link_root = home.path().join(".agents").join("skills");
    let root_meta = std::fs::symlink_metadata(&skills_link_root).expect("skills metadata");
    assert!(root_meta.file_type().is_dir());
    assert!(!root_meta.file_type().is_symlink());
    let skill_link_meta = std::fs::symlink_metadata(skills_link_root.join("orbit-approve-task"))
        .expect("orbit-approve-task link metadata");
    assert!(skill_link_meta.file_type().is_symlink());
}

#[test]
fn init_repairs_broken_per_skill_symlink_targets() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home = tempfile::tempdir().expect("home");

    let skills_link_root = home.path().join(".agents").join("skills");
    std::fs::create_dir_all(&skills_link_root).expect("create skills link root");
    let broken_target = home
        .path()
        .join(".orbit")
        .join("skills")
        .join("does-not-exist");
    create_dir_symlink(&broken_target, &skills_link_root.join("orbit-approve-task"));

    orbit_in(workspace.path())
        .env("HOME", home.path())
        .args(["init"])
        .assert()
        .success();

    let repaired_link = skills_link_root.join("orbit-approve-task");
    let repaired_link_meta = std::fs::symlink_metadata(&repaired_link).expect("repaired metadata");
    assert!(repaired_link_meta.file_type().is_symlink());
    assert!(repaired_link.exists());

    let expected_target = home
        .path()
        .join(".orbit")
        .join("skills")
        .join("orbit-approve-task")
        .canonicalize()
        .expect("canonical expected target");
    let actual_target = repaired_link
        .canonicalize()
        .expect("canonical repaired target");
    assert_eq!(actual_target, expected_target);
}

#[test]
fn init_force_resets_home_orbit_to_defaults() {
    let workspace = tempfile::tempdir().expect("workspace");
    let home = tempfile::tempdir().expect("home");

    let orbit_root = home.path().join(".orbit");
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

    assert!(repo_orbit.join("identities").join("linus.yaml").exists());
    assert!(
        repo_orbit
            .join("skills")
            .join("orbit-approve-task")
            .join("SKILL.md")
            .exists()
    );

    let repo_skills_link_root = repo.path().join(".agents").join("skills");
    let root_meta = std::fs::symlink_metadata(&repo_skills_link_root)
        .expect("repo .agents skills root metadata");
    assert!(root_meta.file_type().is_dir());
    let skill_link_meta =
        std::fs::symlink_metadata(repo_skills_link_root.join("orbit-approve-task"))
            .expect("repo skill symlink metadata");
    assert!(skill_link_meta.file_type().is_symlink());

    let config_raw =
        std::fs::read_to_string(repo_orbit.join("config.toml")).expect("read repo config");
    assert!(config_raw.contains("path = \"skills\""));
    assert!(config_raw.contains("path = \"orbit.db\""));

    let home_orbit = home.path().join(".orbit");
    assert!(home_orbit.join("identities").join("linus.yaml").exists());
    assert!(
        home_orbit
            .join("skills")
            .join("orbit-approve-task")
            .join("SKILL.md")
            .exists()
    );
    assert!(home_orbit.join("config.toml").exists());
}
