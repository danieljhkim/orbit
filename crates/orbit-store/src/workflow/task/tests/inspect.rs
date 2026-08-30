use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use orbit_types::task::{ArtifactManifestV2, TASK_ARTIFACT_SCHEMA_VERSION, TASK_EVENTS_FILE_NAME};
use tempfile::TempDir;

use super::*;

fn metadata(
    workspace_id: &str,
    generation: u64,
    previous: Option<&str>,
) -> PublicationSnapshotMetadata {
    PublicationSnapshotMetadata {
        publication_id: "pub_orbit_primary".to_string(),
        workspace_id: workspace_id.to_string(),
        source_repository_fingerprint: "git@github.com:example/orbit-source.git".to_string(),
        authority_machine_id: "hm_owner".to_string(),
        generation,
        published_at: Utc
            .with_ymd_and_hms(2026, 8, 30, 1, 2, generation as u32)
            .unwrap(),
        previous_publication: previous.map(ToOwned::to_owned),
    }
}

fn policy(kind: AttachmentPolicyKind) -> AttachmentPolicy {
    AttachmentPolicy {
        kind,
        max_file_bytes: 1024,
        max_total_bytes: 4096,
        deny_patterns: Vec::new(),
        scanner_failure_behavior: ScannerFailureBehavior::AllowUnchecked,
    }
}

fn request(
    workspace_id: &str,
    remote: &Path,
    cache: &Path,
    commit: Option<&str>,
) -> PublicationInspectRequest {
    PublicationInspectRequest {
        workspace_id: workspace_id.to_string(),
        source_repository_fingerprint: "git@github.com:example/orbit-source.git".to_string(),
        publication_id: "pub_orbit_primary".to_string(),
        authority_machine_id: "hm_owner".to_string(),
        publication_remote: remote.to_string_lossy().into_owned(),
        publication_branch: "refs/heads/main".to_string(),
        cache_dir: cache.to_path_buf(),
        commit: commit.map(ToOwned::to_owned),
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "orbit-test")
        .env("GIT_AUTHOR_EMAIL", "orbit-test@example.test")
        .env("GIT_COMMITTER_NAME", "orbit-test")
        .env("GIT_COMMITTER_EMAIL", "orbit-test@example.test")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn init_repo(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-b", "main"]);
}

fn copy_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let dest = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).unwrap();
        }
    }
}

fn replace_worktree(repo: &Path, snapshot: &Path) {
    for entry in fs::read_dir(repo).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() == ".git" {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            fs::remove_dir_all(&path).unwrap();
        } else {
            fs::remove_file(&path).unwrap();
        }
    }
    copy_tree(snapshot, repo);
}

fn commit_snapshot(repo: &Path, snapshot: &Path, message: &str) -> String {
    replace_worktree(repo, snapshot);
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-m", message]);
    git(repo, &["rev-parse", "HEAD"])
}

fn amend_current(repo: &Path) {
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "--amend", "--no-edit"]);
}

fn tree_bytes(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, path: &Path, output: &mut BTreeMap<String, Vec<u8>>) {
        let mut entries: Vec<_> = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap())
            .collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if entry.file_name() == ".git" {
                continue;
            }
            let path = entry.path();
            let file_type = entry.file_type().unwrap();
            if file_type.is_dir() || (file_type.is_symlink() && path.is_dir()) {
                visit(root, &path, output);
            } else if file_type.is_file() {
                output.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut output = BTreeMap::new();
    visit(root, root, &mut output);
    output
}

fn seed_one(
    store: &TaskBundleStoreV2,
    registry: &TaskRegistryStore,
    workspace_id: &str,
    task_id: &str,
    files: &[(&str, &[u8])],
) {
    if files.is_empty() {
        seed(
            store,
            registry,
            workspace_id,
            &make_bundle(task_id, task_id, Vec::new()),
        );
        return;
    }
    seed(
        store,
        registry,
        workspace_id,
        &make_bundle(task_id, task_id, Vec::new()),
    );
    let entries: Vec<_> = files
        .iter()
        .map(|(path, bytes)| seed_artifact_blob(store, task_id, path, bytes, "codex"))
        .collect();
    store
        .rewrite_artifact_manifest(
            task_id,
            &ArtifactManifestV2 {
                schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                files: entries,
            },
        )
        .unwrap();
}

struct LinearRepo {
    root: TempDir,
    remote: PathBuf,
    cache: PathBuf,
    source_checkout: PathBuf,
    workspace_id: String,
    gen1: String,
    gen2: String,
}

fn linear_repo(workspace_id: &str, kind: AttachmentPolicyKind) -> LinearRepo {
    let root = TempDir::new().unwrap();
    let registry = open_registry(root.path());
    let binding = bind(&registry, root.path(), workspace_id);
    let store = bundle_store(&registry, &binding);
    seed_one(
        &store,
        &registry,
        workspace_id,
        "ORB-00001",
        if kind == AttachmentPolicyKind::Fail {
            &[]
        } else {
            &[("notes.txt", b"hello")]
        },
    );

    let snap1 = root.path().join("snap-1");
    build_publication_snapshot(
        &registry,
        &snap1,
        metadata(workspace_id, 1, None),
        &policy(kind),
        None,
    )
    .unwrap();
    let remote = root.path().join("publication.git");
    init_repo(&remote);
    let gen1 = commit_snapshot(&remote, &snap1, "generation 1");

    let snap2 = root.path().join("snap-2");
    build_publication_snapshot(
        &registry,
        &snap2,
        metadata(workspace_id, 2, Some(&gen1)),
        &policy(kind),
        None,
    )
    .unwrap();
    let gen2 = commit_snapshot(&remote, &snap2, "generation 2");

    let source_checkout = root.path().join("source-checkout");
    init_repo(&source_checkout);
    fs::write(source_checkout.join("README.md"), "source\n").unwrap();
    git(&source_checkout, &["add", "README.md"]);
    git(&source_checkout, &["commit", "-m", "source"]);

    LinearRepo {
        cache: root.path().join("consumer-cache"),
        remote,
        source_checkout,
        workspace_id: workspace_id.to_string(),
        gen1,
        gen2,
        root,
    }
}

fn assert_label(
    inspection: &PublicationInspection,
    workspace_id: &str,
    generation: u64,
    commit: &str,
    freshness: PublicationFreshness,
    incomplete: bool,
) {
    let label = &inspection.label;
    assert_eq!(label.workspace_id, workspace_id);
    assert_eq!(label.generation, generation);
    assert_eq!(label.commit_id, commit);
    assert_eq!(
        label.source_repository_fingerprint,
        "git@github.com:example/orbit-source.git"
    );
    assert_eq!(label.authority_machine_id, "hm_owner");
    assert_eq!(label.publication_id, "pub_orbit_primary");
    assert_eq!(label.freshness, freshness);
    assert_eq!(label.incomplete_attachments, incomplete);
    assert_eq!(label.render_authority, PublicationRenderAuthority::Snapshot);
    assert!(!inspection.tasks.is_empty());
    for task in &inspection.tasks {
        assert_eq!(task.label, *label);
        assert_eq!(
            task.label.render_authority,
            PublicationRenderAuthority::Snapshot
        );
    }
}

#[test]
fn current_snapshot_is_labelled_and_not_live() {
    let repo = linear_repo("ws_inspect_current", AttachmentPolicyKind::Fail);
    let inspection =
        inspect_publication(request(&repo.workspace_id, &repo.remote, &repo.cache, None)).unwrap();
    assert_label(
        &inspection,
        &repo.workspace_id,
        2,
        &repo.gen2,
        PublicationFreshness::Current,
        false,
    );
    assert_eq!(inspection.git_parent.as_deref(), Some(repo.gen1.as_str()));
    assert_eq!(inspection.tasks[0].task.id, "ORB-00001");
}

#[test]
fn stale_snapshot_is_labelled_with_older_generation() {
    let repo = linear_repo("ws_inspect_stale", AttachmentPolicyKind::Fail);
    let inspection = inspect_publication(request(
        &repo.workspace_id,
        &repo.remote,
        &repo.cache,
        Some(&repo.gen1),
    ))
    .unwrap();
    assert_label(
        &inspection,
        &repo.workspace_id,
        1,
        &repo.gen1,
        PublicationFreshness::Stale,
        false,
    );
    assert_eq!(inspection.git_parent, None);
}

#[test]
fn omit_projection_is_labelled_incomplete() {
    let repo = linear_repo("ws_inspect_omit", AttachmentPolicyKind::Omit);
    let inspection =
        inspect_publication(request(&repo.workspace_id, &repo.remote, &repo.cache, None)).unwrap();
    assert_label(
        &inspection,
        &repo.workspace_id,
        2,
        &repo.gen2,
        PublicationFreshness::Current,
        true,
    );
    assert!(!inspection.envelope.omitted_attachments.is_empty());
}

#[test]
fn tampered_bundle_and_jsonl_fail_before_trusted_state() {
    let repo = linear_repo("ws_inspect_tamper", AttachmentPolicyKind::Include);
    let blob = repo
        .remote
        .join("tasks/ORB-00001/artifacts/files/notes.txt");
    fs::write(&blob, b"tampered-secret-content").unwrap();
    amend_current(&repo.remote);
    let error = inspect_publication(request(&repo.workspace_id, &repo.remote, &repo.cache, None))
        .unwrap_err()
        .to_string();
    assert!(error.contains("ORB-00001"));
    assert!(!error.contains("tampered-secret-content"));

    git(&repo.remote, &["reset", "--hard", &repo.gen2]);
    let events = repo
        .remote
        .join("tasks/ORB-00001")
        .join(TASK_EVENTS_FILE_NAME);
    let raw = fs::read_to_string(&events).unwrap();
    fs::write(&events, format!("{raw}{{")).unwrap();
    amend_current(&repo.remote);
    let jsonl_error = inspect_publication(request(
        &repo.workspace_id,
        &repo.remote,
        &repo.cache.join("jsonl"),
        None,
    ))
    .unwrap_err()
    .to_string();
    assert!(jsonl_error.contains("ORB-00001"));
    assert!(jsonl_error.contains("events.jsonl"));
}

#[test]
fn mismatched_repository_and_branch_fail_closed() {
    let repo = linear_repo("ws_inspect_mismatch", AttachmentPolicyKind::Fail);
    let mut wrong_workspace = request(&repo.workspace_id, &repo.remote, &repo.cache, None);
    wrong_workspace.workspace_id = "ws_other".to_string();
    assert!(
        inspect_publication(wrong_workspace)
            .unwrap_err()
            .to_string()
            .contains("workspace mismatch")
    );

    let mut wrong_source = request(
        &repo.workspace_id,
        &repo.remote,
        &repo.cache.join("source"),
        None,
    );
    wrong_source.source_repository_fingerprint =
        "git@github.com:example/other-source.git".to_string();
    assert!(
        inspect_publication(wrong_source)
            .unwrap_err()
            .to_string()
            .contains("source repository fingerprint mismatch")
    );

    let mut wrong_lineage = request(
        &repo.workspace_id,
        &repo.remote,
        &repo.cache.join("lineage"),
        None,
    );
    wrong_lineage.publication_id = "pub_other".to_string();
    assert!(
        inspect_publication(wrong_lineage)
            .unwrap_err()
            .to_string()
            .contains("publication id mismatch")
    );

    let mut wrong_authority = request(
        &repo.workspace_id,
        &repo.remote,
        &repo.cache.join("authority"),
        None,
    );
    wrong_authority.authority_machine_id = "hm_other".to_string();
    assert!(
        inspect_publication(wrong_authority)
            .unwrap_err()
            .to_string()
            .contains("authority mismatch")
    );

    let mut wrong_branch = request(
        &repo.workspace_id,
        &repo.remote,
        &repo.cache.join("branch"),
        None,
    );
    wrong_branch.publication_branch = "refs/heads/other".to_string();
    let branch_error = inspect_publication(wrong_branch).unwrap_err().to_string();
    assert!(
        branch_error.contains("branch") || branch_error.contains("git"),
        "{branch_error}"
    );
}

#[test]
fn unsupported_future_schemas_fail_closed() {
    let repo = linear_repo("ws_inspect_schema", AttachmentPolicyKind::Fail);
    let envelope_path = repo.remote.join(PUBLICATION_ENVELOPE_FILE_NAME);
    let yaml = fs::read_to_string(&envelope_path)
        .unwrap()
        .replace("format_version: 1", "format_version: 9");
    fs::write(&envelope_path, yaml).unwrap();
    amend_current(&repo.remote);
    let envelope_error =
        inspect_publication(request(&repo.workspace_id, &repo.remote, &repo.cache, None))
            .unwrap_err()
            .to_string();
    assert!(envelope_error.contains("unsupported task publication format version 9"));

    git(&repo.remote, &["reset", "--hard", &repo.gen2]);
    let task_yaml = repo.remote.join("tasks/ORB-00001/task.yaml");
    fs::write(
        &task_yaml,
        "schema_version: 999\nid: ORB-00001\ntitle: future\n",
    )
    .unwrap();
    amend_current(&repo.remote);
    let task_error = inspect_publication(request(
        &repo.workspace_id,
        &repo.remote,
        &repo.cache.join("task-schema"),
        None,
    ))
    .unwrap_err()
    .to_string();
    assert!(task_error.contains("ORB-00001"));
}

#[test]
fn parent_mismatch_fails_closed() {
    let repo = linear_repo("ws_inspect_parent", AttachmentPolicyKind::Fail);
    let envelope_path = repo.remote.join(PUBLICATION_ENVELOPE_FILE_NAME);
    let yaml = fs::read_to_string(&envelope_path)
        .unwrap()
        .replace(&repo.gen1, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    fs::write(&envelope_path, yaml).unwrap();
    git(&repo.remote, &["add", "-A"]);
    git(&repo.remote, &["commit", "-m", "wrong parent"]);
    let error = inspect_publication(request(&repo.workspace_id, &repo.remote, &repo.cache, None))
        .unwrap_err()
        .to_string();
    assert!(error.contains("previous-publication"));
}

#[test]
fn inspect_does_not_mutate_canonical_or_source_state() {
    let repo = linear_repo("ws_inspect_readonly", AttachmentPolicyKind::Fail);
    let registry = open_registry(repo.root.path());
    let before_tasks = registry
        .tasks_for_workspace(&repo.workspace_id)
        .unwrap()
        .len();
    let before_allocator = registry.allocator_next_number().unwrap();
    let canonical = registry
        .canonical_task_bundle_path(&repo.workspace_id, "ORB-00001")
        .unwrap();
    let before_bundle = tree_bytes(&canonical);
    let before_source = tree_bytes(&repo.source_checkout);
    let before_source_head = git(&repo.source_checkout, &["rev-parse", "HEAD"]);
    let before_source_status = git(&repo.source_checkout, &["status", "--porcelain"]);
    let checkout = repo
        .root
        .path()
        .join("repos")
        .join(&repo.workspace_id)
        .join(".orbit");
    let before_checkout = tree_bytes(&checkout);

    inspect_publication(request(&repo.workspace_id, &repo.remote, &repo.cache, None)).unwrap();

    assert_eq!(
        registry
            .tasks_for_workspace(&repo.workspace_id)
            .unwrap()
            .len(),
        before_tasks
    );
    assert_eq!(registry.allocator_next_number().unwrap(), before_allocator);
    assert_eq!(tree_bytes(&canonical), before_bundle);
    assert_eq!(tree_bytes(&repo.source_checkout), before_source);
    assert_eq!(
        git(&repo.source_checkout, &["rev-parse", "HEAD"]),
        before_source_head
    );
    assert_eq!(
        git(&repo.source_checkout, &["status", "--porcelain"]),
        before_source_status
    );
    assert_eq!(tree_bytes(&checkout), before_checkout);
    assert!(!repo.root.path().join("claims").exists());
    assert!(!repo.root.path().join("audit").exists());
    assert!(!repo.root.path().join("runs").exists());
}
