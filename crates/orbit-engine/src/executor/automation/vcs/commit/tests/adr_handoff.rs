//! The delivery step's host-side staging handoff for proposed ADR bundles.
//!
//! A workspace that keeps `.orbit/adrs/proposed/` ignored — the shape written
//! by workspace init — would otherwise let `git add --all` drop a draft that
//! documents the very code being delivered. These tests pin the handoff, its
//! `proposed/`-only scope, and the refusal it produces when the worktree's git
//! metadata is read-only, which is exactly the linked-worktree case the
//! sandboxed implementer cannot work around.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::super::git_commit;
use super::test_support::*;

use super::super::super::git::{git_output, git_success};

const TASK_ID: &str = "ORB-ADR-HANDOFF";
const BUNDLE_ID: &str = "ADR-0777";

fn batch_input(workspace: &Path) -> Value {
    json!({
        "scope": "all",
        "job_run_id": "batch-1",
        "workspace_path": workspace.to_string_lossy().to_string(),
    })
}

/// Ignore the proposed partition the way workspace init does, and commit that
/// rule so a linked worktree inherits it.
fn ignore_proposed_partition(repo: &Path) {
    fs::write(
        repo.join(".gitignore"),
        ".orbit/*\n!.orbit/adrs/\n.orbit/adrs/proposed/\n.orbit/adrs/superseded/\n",
    )
    .expect("write gitignore");
    git_success(repo, &["add", ".gitignore"]).expect("stage gitignore");
    git_success(repo, &["commit", "-m", "ignore local-only ADR partitions"])
        .expect("commit gitignore");
}

/// Write one ADR bundle under `partition` as the allocator would.
fn write_bundle(workspace: &Path, partition: &str, id: &str) {
    let dir = workspace.join(".orbit/adrs").join(partition).join(id);
    fs::create_dir_all(&dir).expect("bundle dir");
    fs::write(
        dir.join("adr.yaml"),
        format!("id: {id}\nstatus: {partition}\n"),
    )
    .expect("write adr.yaml");
    fs::write(
        dir.join("body.md"),
        "## Context\nc\n## Decision\nd\n## Consequences\n- Cost: c\n",
    )
    .expect("write body.md");
}

/// The code change the run is delivering alongside its ADR.
fn write_code_change(workspace: &Path) {
    fs::create_dir_all(workspace.join("src")).expect("src dir");
    fs::write(workspace.join("src/change.txt"), "delivered work\n").expect("write change");
}

fn host_for(workspace: &Path) -> CommitTestHost {
    CommitTestHost::new(
        vec![task_with_file(
            TASK_ID,
            "Deliver code with its proposed ADR",
            "src/change.txt",
            "codex",
        )],
        workspace.to_path_buf(),
    )
}

fn committed_files(workspace: &Path) -> Vec<String> {
    git_output(
        workspace,
        &["show", "--name-only", "--pretty=format:", "HEAD"],
    )
    .expect("read commit file list")
    .lines()
    .map(str::trim)
    .filter(|line| !line.is_empty())
    .map(ToOwned::to_owned)
    .collect()
}

#[test]
fn ignored_proposed_bundle_is_force_staged_into_the_delivery_commit() {
    let temp = initialized_git_repo();
    let workspace = temp.path();
    ignore_proposed_partition(workspace);
    write_bundle(workspace, "proposed", BUNDLE_ID);
    write_code_change(workspace);

    // Precondition: the bundle really is invisible to the normal staging path.
    git_success(workspace, &["add", "--all", "--", "."]).expect("baseline stage");
    let staged = git_output(workspace, &["diff", "--cached", "--name-only"]).expect("baseline");
    assert!(
        !staged.contains(BUNDLE_ID),
        "precondition: `git add --all` must not reach the ignored bundle, got: {staged}"
    );
    git_success(workspace, &["reset"]).expect("undo baseline stage");

    git_commit(&host_for(workspace), &batch_input(workspace)).expect("delivery succeeds");

    let files = committed_files(workspace);
    for name in ["adr.yaml", "body.md"] {
        let expected = format!(".orbit/adrs/proposed/{BUNDLE_ID}/{name}");
        assert!(
            files.contains(&expected),
            "handoff must deliver {expected}, commit held: {files:?}"
        );
    }
    assert!(
        files.contains(&"src/change.txt".to_string()),
        "the code change still ships: {files:?}"
    );
}

#[test]
fn handoff_leaves_accepted_and_superseded_partitions_to_their_own_rules() {
    let temp = initialized_git_repo();
    let workspace = temp.path();
    ignore_proposed_partition(workspace);
    write_bundle(workspace, "accepted", "ADR-0778");
    write_bundle(workspace, "superseded", "ADR-0779");
    write_code_change(workspace);

    git_commit(&host_for(workspace), &batch_input(workspace)).expect("delivery succeeds");

    let files = committed_files(workspace);
    assert!(
        files.iter().any(|file| file.contains("ADR-0778")),
        "accepted ADRs are re-included by the gitignore and ship unchanged: {files:?}"
    );
    assert!(
        !files.iter().any(|file| file.contains("ADR-0779")),
        "superseded stays ignored; the handoff must not force-stage it: {files:?}"
    );
}

#[test]
fn unignored_proposed_bundle_needs_no_handoff() {
    // A workspace that tracks its proposed partition is already covered by
    // `git add --all`; the handoff must be inert rather than double-staging.
    let temp = initialized_git_repo();
    let workspace = temp.path();
    write_bundle(workspace, "proposed", BUNDLE_ID);
    write_code_change(workspace);

    git_commit(&host_for(workspace), &batch_input(workspace)).expect("delivery succeeds");

    let files = committed_files(workspace);
    assert!(
        files.contains(&format!(".orbit/adrs/proposed/{BUNDLE_ID}/body.md")),
        "an unignored bundle ships through the normal path: {files:?}"
    );
}

/// `chmod` is advisory for a process that bypasses permission checks, so the
/// read-only fixture below cannot be constructed as root.
#[cfg(unix)]
fn running_as_root() -> bool {
    // SAFETY: geteuid has no preconditions and only reads the process effective uid.
    unsafe { libc::geteuid() == 0 }
}

/// Restore a directory's mode on drop so a failing assertion still leaves the
/// tempdir deletable.
#[cfg(unix)]
struct RestoreMode {
    path: PathBuf,
    mode: u32,
}

#[cfg(unix)]
impl Drop for RestoreMode {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;

        let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(self.mode));
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> RestoreMode {
    use std::os::unix::fs::PermissionsExt;

    let previous = fs::metadata(path)
        .expect("mode metadata")
        .permissions()
        .mode()
        & 0o7777;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("set mode");
    RestoreMode {
        path: path.to_path_buf(),
        mode: previous,
    }
}

#[cfg(unix)]
#[test]
fn read_only_linked_worktree_metadata_refuses_and_names_the_bundle() {
    if running_as_root() {
        // Not an incidental skip: as root the fixture cannot express the
        // condition under test at all.
        return;
    }

    let temp = initialized_git_repo();
    let main_repo = temp.path();
    ignore_proposed_partition(main_repo);

    // A linked worktree keeps its index under the main checkout's
    // `.git/worktrees/<name>/`, which is the directory bound read-only for a
    // sandboxed implementer.
    let linked = main_repo.join("linked-worktree");
    git_success(
        main_repo,
        &[
            "worktree",
            "add",
            "-b",
            "task-branch",
            &linked.to_string_lossy(),
        ],
    )
    .expect("add linked worktree");

    write_bundle(&linked, "proposed", BUNDLE_ID);
    write_code_change(&linked);

    let metadata_dir = main_repo.join(".git/worktrees/linked-worktree");
    assert!(
        metadata_dir.is_dir(),
        "precondition: linked worktree metadata lives in the main checkout"
    );
    let _restore = set_mode(&metadata_dir, 0o555);

    let error = git_commit(&host_for(&linked), &batch_input(&linked))
        .expect_err("delivery must refuse rather than drop the bundle");
    let message = error.to_string();

    assert!(
        message.contains(TASK_ID),
        "refusal names the task: {message}"
    );
    assert!(
        message.contains(&format!(".orbit/adrs/proposed/{BUNDLE_ID}/adr.yaml")),
        "refusal names the unstaged bundle: {message}"
    );
    assert!(
        message.contains("Supported path:") && message.contains("host-side"),
        "refusal names the supported handoff: {message}"
    );
    assert!(
        message.contains("invent an ADR id"),
        "refusal rules out fabricating an id: {message}"
    );

    assert_eq!(
        git_output(&linked, &["rev-list", "--count", "HEAD"])
            .expect("commit count")
            .trim(),
        "2",
        "refusing delivery creates no commit"
    );
}
