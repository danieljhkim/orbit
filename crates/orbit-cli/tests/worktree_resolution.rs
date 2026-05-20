#![allow(missing_docs)]
// ORB-00013: Tests use unwrap/expect to keep fixture setup readable.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

use assert_cmd::Command as AssertCommand;
use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::{Value, json};
use tempfile::tempdir;

#[test]
fn config_show_reports_shared_and_local_roots_for_git_worktrees_and_overrides() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let main_repo = temp.path().join("repo");
    let linked_worktree = temp.path().join("repo-worktree");
    fs::create_dir_all(&home).expect("create home");
    fs::create_dir_all(&main_repo).expect("create main repo");

    run_git(&main_repo, &["init"]);
    run_git(&main_repo, &["config", "user.name", "Orbit Test"]);
    run_git(
        &main_repo,
        &["config", "user.email", "orbit-test@example.com"],
    );
    run_git(&main_repo, &["config", "commit.gpgsign", "false"]);
    fs::write(main_repo.join("README.md"), "# orbit\n").expect("write readme");
    run_git(&main_repo, &["add", "README.md"]);
    run_git(&main_repo, &["commit", "-m", "initial"]);
    run_git(
        &main_repo,
        &[
            "worktree",
            "add",
            "-b",
            "orbit-worktree-resolution",
            linked_worktree.to_str().expect("utf8 worktree path"),
        ],
    );

    run_orbit_success(&main_repo, &home, &["workspace", "init"], None);

    let main_repo = fs::canonicalize(&main_repo).expect("canonicalize main repo");
    let linked_worktree = fs::canonicalize(&linked_worktree).expect("canonicalize linked worktree");
    let main_orbit = main_repo.join(".orbit");
    let linked_orbit = linked_worktree.join(".orbit");

    let from_main = run_orbit_json(&main_repo, &home, &["config", "show", "--json"], None);
    assert_root_fields(&from_main, &main_orbit, &main_orbit);

    let from_worktree =
        run_orbit_json(&linked_worktree, &home, &["config", "show", "--json"], None);
    assert_root_fields(&from_worktree, &main_orbit, &linked_orbit);
    assert!(
        !linked_orbit.exists(),
        "linked worktree local root should be resolved but not created"
    );

    let explicit_root = main_orbit.to_string_lossy().to_string();
    let from_root_override = run_orbit_json(
        &linked_worktree,
        &home,
        &["--root", &explicit_root, "config", "show", "--json"],
        None,
    );
    assert_root_fields(&from_root_override, &main_orbit, &main_orbit);

    let from_env = run_orbit_json(
        &linked_worktree,
        &home,
        &["config", "show", "--json"],
        Some(&main_orbit),
    );
    assert_root_fields(&from_env, &main_orbit, &main_orbit);
    assert!(
        !linked_orbit.exists(),
        "resolution should not materialize a linked-worktree .orbit directory"
    );
}

#[test]
fn linked_worktree_artifacts_write_locally_and_remote_lists_return_stubs() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let main_repo = temp.path().join("repo");
    let linked_worktree = temp.path().join("repo-artifacts");
    fs::create_dir_all(&home).expect("create home");
    fs::create_dir_all(&main_repo).expect("create main repo");

    init_git_repo(&main_repo);
    run_git(
        &main_repo,
        &[
            "worktree",
            "add",
            "-b",
            "orbit-worktree-artifacts",
            linked_worktree.to_str().expect("utf8 worktree path"),
        ],
    );
    run_orbit_success(&main_repo, &home, &["workspace", "init"], None);

    let main_repo = fs::canonicalize(&main_repo).expect("canonicalize main repo");
    let linked_worktree = fs::canonicalize(&linked_worktree).expect("canonicalize linked worktree");
    let main_orbit = main_repo.join(".orbit");
    let linked_orbit = linked_worktree.join(".orbit");

    let adr = run_orbit_json(
        &linked_worktree,
        &home,
        &[
            "tool",
            "run",
            "orbit.adr.add",
            "--input",
            r###"{"title":"Worktree artifact routing","owner":"codex","body":"## Context\nLinked worktree write.\n\n## Decision\nWrite locally.\n\n## Consequences\n- Body files are committable.\n- Cost: Readers need federation.\n","related_features":["worktree-artifacts"],"related_tasks":["ORB-00201"],"model":"codex"}"###,
        ],
        None,
    );
    let adr_id = string_field(&adr, "id").to_string();

    let learning = run_orbit_json(
        &linked_worktree,
        &home,
        &[
            "learning",
            "add",
            "--summary",
            "Stage worktree-local learning artifacts with the code change",
            "--path",
            "crates/**",
            "--tag",
            "worktree-artifacts",
            "--evidence",
            "task:ORB-00201",
            "--json",
        ],
        None,
    );
    let learning_id = string_field(&learning, "id").to_string();
    assert!(learning_id.starts_with("L-"));

    let adr_dir = linked_orbit.join("adrs/proposed").join(&adr_id);
    assert!(adr_dir.join("adr.yaml").is_file());
    assert!(adr_dir.join("body.md").is_file());
    assert!(!main_orbit.join("adrs/proposed").join(&adr_id).exists());

    let learning_dir = linked_orbit.join("learnings").join(&learning_id);
    assert!(learning_dir.join("learning.yaml").is_file());
    assert!(learning_dir.join("votes.jsonl").is_file());
    assert!(learning_dir.join("comments.jsonl").is_file());
    assert!(!main_orbit.join("learnings").join(&learning_id).exists());

    let local_orbit_entries = sorted_child_names(&linked_orbit);
    assert_eq!(local_orbit_entries, vec!["adrs", "learnings"]);

    run_git(
        &main_repo,
        &[
            "worktree",
            "remove",
            "--force",
            linked_worktree.to_str().expect("utf8 worktree path"),
        ],
    );

    let adr_default = run_orbit_json(
        &main_repo,
        &home,
        &[
            "tool",
            "run",
            "orbit.adr.list",
            "--input",
            r#"{"model":"codex"}"#,
        ],
        None,
    );
    assert!(!array_contains_id(&adr_default, &adr_id));

    let adr_remote = run_orbit_json(
        &main_repo,
        &home,
        &[
            "tool",
            "run",
            "orbit.adr.list",
            "--input",
            r#"{"include_remote":true,"model":"codex"}"#,
        ],
        None,
    );
    let adr_stub = find_id(&adr_remote, &adr_id);
    assert_eq!(adr_stub["remote"], json!(true));
    assert!(
        adr_stub["remote_marker"]
            .as_str()
            .expect("marker")
            .contains("[remote:")
    );

    let learning_default = run_orbit_json(&main_repo, &home, &["learning", "list", "--json"], None);
    assert!(!array_contains_id(&learning_default, &learning_id));

    let learning_remote = run_orbit_json(
        &main_repo,
        &home,
        &["learning", "list", "--include-remote", "--json"],
        None,
    );
    let learning_stub = find_id(&learning_remote, &learning_id);
    assert_eq!(learning_stub["remote"], json!(true));
    assert!(learning_stub["body"].is_null());

    let show_output = run_orbit_output(
        &main_repo,
        &home,
        &[
            "tool",
            "run",
            "orbit.adr.show",
            "--input",
            &format!(r#"{{"id":"{adr_id}","model":"codex"}}"#),
        ],
        None,
    );
    assert!(!show_output.status.success());
    let output_text = format!(
        "{}{}",
        String::from_utf8_lossy(&show_output.stdout),
        String::from_utf8_lossy(&show_output.stderr)
    );
    assert!(
        output_text.contains("worktree_root="),
        "output: {output_text}"
    );
    assert!(
        output_text.contains("orbit-worktree-artifacts"),
        "output: {output_text}"
    );
}

fn assert_root_fields(value: &Value, shared_root: &Path, local_root: &Path) {
    let shared = shared_root.to_string_lossy();
    let local = local_root.to_string_lossy();
    assert_eq!(string_field(value, "shared_root"), shared.as_ref());
    assert_eq!(string_field(value, "local_root"), local.as_ref());
    assert!(
        value.get("workspace_root").is_none(),
        "legacy `workspace_root` alias must be removed from `config show --json` output (use `shared_root`)"
    );
    assert!(
        value.get("root").is_none(),
        "legacy `root` alias must be removed from `config show --json` output (use `shared_root`)"
    );
    assert!(
        value.get("selected_root").is_none(),
        "legacy `selected_root` alias must be removed from `config show --json` output (use `shared_root`)"
    );
}

fn string_field<'a>(value: &'a Value, field: &str) -> &'a str {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("expected string field `{field}` in {value}"))
}

fn run_orbit_success(cwd: &Path, home: &Path, args: &[&str], orbit_root: Option<&Path>) {
    let mut command = cargo_bin_cmd!("orbit");
    command
        .current_dir(cwd)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .args(args);
    set_orbit_root_env(&mut command, orbit_root);
    command.assert().success();
}

fn run_orbit_json(cwd: &Path, home: &Path, args: &[&str], orbit_root: Option<&Path>) -> Value {
    let mut command = cargo_bin_cmd!("orbit");
    command
        .current_dir(cwd)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .args(args);
    set_orbit_root_env(&mut command, orbit_root);
    let assert = command.assert().success();
    serde_json::from_slice(&assert.get_output().stdout).expect("orbit json output")
}

fn run_orbit_output(
    cwd: &Path,
    home: &Path,
    args: &[&str],
    orbit_root: Option<&Path>,
) -> std::process::Output {
    let mut command = cargo_bin_cmd!("orbit");
    command
        .current_dir(cwd)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .args(args);
    set_orbit_root_env(&mut command, orbit_root);
    command.output().expect("run orbit")
}

fn set_orbit_root_env(command: &mut AssertCommand, orbit_root: Option<&Path>) {
    match orbit_root {
        Some(path) => {
            command.env("ORBIT_ROOT", path);
        }
        None => {
            command.env_remove("ORBIT_ROOT");
        }
    }
}

fn run_git(cwd: &Path, args: &[&str]) {
    let output = StdCommand::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git -C {} {} failed\nstdout:\n{}\nstderr:\n{}",
        cwd.display(),
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_git_repo(main_repo: &Path) {
    run_git(main_repo, &["init"]);
    run_git(main_repo, &["config", "user.name", "Orbit Test"]);
    run_git(
        main_repo,
        &["config", "user.email", "orbit-test@example.com"],
    );
    run_git(main_repo, &["config", "commit.gpgsign", "false"]);
    fs::write(main_repo.join("README.md"), "# orbit\n").expect("write readme");
    run_git(main_repo, &["add", "README.md"]);
    run_git(main_repo, &["commit", "-m", "initial"]);
}

fn sorted_child_names(path: &Path) -> Vec<String> {
    let mut entries = fs::read_dir(path)
        .expect("read dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_str()
                .expect("utf8")
                .to_string()
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

fn array_contains_id(value: &Value, id: &str) -> bool {
    value
        .as_array()
        .expect("array")
        .iter()
        .any(|item| item.get("id").and_then(Value::as_str) == Some(id))
}

fn find_id<'a>(value: &'a Value, id: &str) -> &'a Value {
    value
        .as_array()
        .expect("array")
        .iter()
        .find(|item| item.get("id").and_then(Value::as_str) == Some(id))
        .unwrap_or_else(|| panic!("missing id {id} in {value}"))
}
