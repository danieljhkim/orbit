use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Barrier, mpsc};
use std::thread;
use std::time::Duration;

use fs2::FileExt;
use orbit_knowledge::graph::nodes::{
    BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, LeafKind, LeafNode,
};
use orbit_knowledge::graph::object_store::{GraphObjectStore, RefName, resolve_graph_read_target};
use orbit_knowledge::pipeline::context::BuildConfig;
use orbit_knowledge::pipeline::{RefreshStatus, ensure_fresh, run_build};
use orbit_knowledge::{KnowledgeStore, Selector};
use tempfile::TempDir;

#[test]
fn knowledge_store_open_migrates_legacy_current_ref_to_default_branch()
-> Result<(), Box<dyn std::error::Error>> {
    let repo = init_repo()?;
    let knowledge_dir = repo.path().join(".orbit/knowledge");
    let graph_store = GraphObjectStore::new(knowledge_dir.join("graph"));
    let current_ref = graph_store.write_graph(&sample_graph("main_graph"))?;
    write_manifest(&knowledge_dir)?;
    write_legacy_current_ref(&knowledge_dir, &current_ref)?;

    let read_target = resolve_graph_read_target(Some(repo.path()), None)?;
    let store = KnowledgeStore::open(
        &knowledge_dir,
        &read_target.requested,
        read_target.fallback.as_ref(),
        read_target.default.as_ref(),
    )?;

    let selector: Selector = "symbol:src/lib.rs#main_graph:function".parse()?;
    let pack = store.pack(&[selector])?;
    assert_eq!(pack.total_nodes, 1);
    assert!(!knowledge_dir.join("graph/refs/current.json").exists());
    assert!(knowledge_dir.join("graph/refs/heads/main.json").is_file());

    let second = KnowledgeStore::open(
        &knowledge_dir,
        &read_target.requested,
        read_target.fallback.as_ref(),
        read_target.default.as_ref(),
    )?;
    let second_pack = second.pack(&["symbol:src/lib.rs#main_graph:function".parse()?])?;
    assert_eq!(second_pack.total_nodes, 1);
    Ok(())
}

#[test]
fn graph_reads_fall_back_to_default_branch_when_current_branch_ref_is_missing()
-> Result<(), Box<dyn std::error::Error>> {
    let repo = init_repo()?;
    let knowledge_dir = repo.path().join(".orbit/knowledge");
    let graph_store = GraphObjectStore::new(knowledge_dir.join("graph"));

    let main_ref = RefName::new("main")?;
    graph_store.prepare_refs_layout(Some(&main_ref))?;
    let current_ref = graph_store.write_graph(&sample_graph("main_graph"))?;
    graph_store.write_ref_atomic(&main_ref, &current_ref)?;

    git(repo.path(), &["checkout", "-b", "feature/missing-ref"])?;
    let read_target = resolve_graph_read_target(Some(repo.path()), None)?;
    assert_eq!(read_target.requested.as_str(), "feature/missing-ref");
    assert_eq!(
        read_target.fallback.as_ref().map(RefName::as_str),
        Some("main")
    );

    let graph = graph_store.read_graph(
        &read_target.requested,
        read_target.fallback.as_ref(),
        read_target.default.as_ref(),
    )?;
    assert_eq!(graph.root_dir_id, "dir:.");
    assert_eq!(graph.leaves.len(), 1);
    assert_eq!(graph.leaves[0].base.name, "main_graph");
    Ok(())
}

#[test]
fn graph_reads_fall_back_to_non_main_default_branch_when_current_branch_ref_is_missing()
-> Result<(), Box<dyn std::error::Error>> {
    let repo = init_repo_with_default("trunk")?;
    let knowledge_dir = repo.path().join(".orbit/knowledge");
    let graph_store = GraphObjectStore::new(knowledge_dir.join("graph"));

    let trunk_ref = RefName::new("trunk")?;
    graph_store.prepare_refs_layout(Some(&trunk_ref))?;
    let current_ref = graph_store.write_graph(&sample_graph("trunk_graph"))?;
    graph_store.write_ref_atomic(&trunk_ref, &current_ref)?;

    git(repo.path(), &["checkout", "-b", "feature/missing-ref"])?;
    let read_target = resolve_graph_read_target(Some(repo.path()), None)?;
    assert_eq!(read_target.requested.as_str(), "feature/missing-ref");
    assert_eq!(
        read_target.fallback.as_ref().map(RefName::as_str),
        Some("trunk")
    );
    assert_eq!(
        read_target.default.as_ref().map(RefName::as_str),
        Some("trunk")
    );

    let graph = graph_store.read_graph(
        &read_target.requested,
        read_target.fallback.as_ref(),
        read_target.default.as_ref(),
    )?;
    assert_eq!(graph.root_dir_id, "dir:.");
    assert_eq!(graph.leaves.len(), 1);
    assert_eq!(graph.leaves[0].base.name, "trunk_graph");
    Ok(())
}

#[test]
fn concurrent_branch_builds_keep_distinct_refs_and_graphs_reachable()
-> Result<(), Box<dyn std::error::Error>> {
    let repo = init_repo()?;
    git(repo.path(), &["branch", "feature/alpha"])?;
    git(repo.path(), &["branch", "feature/beta"])?;

    let alpha_worktree = repo.path().join("wt-alpha");
    let beta_worktree = repo.path().join("wt-beta");
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            alpha_worktree.to_str().unwrap(),
            "feature/alpha",
        ],
    )?;
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            beta_worktree.to_str().unwrap(),
            "feature/beta",
        ],
    )?;
    write_rust_source(&alpha_worktree, "alpha_graph")?;
    commit_all_with_date(&alpha_worktree, "alpha graph", "2030-01-02T00:00:00Z")?;
    write_rust_source(&beta_worktree, "beta_graph")?;
    commit_all_with_date(&beta_worktree, "beta graph", "2030-01-03T00:00:00Z")?;

    let knowledge_dir = repo.path().join(".orbit/knowledge");
    let alpha_knowledge_dir = knowledge_dir.clone();
    let beta_knowledge_dir = knowledge_dir.clone();
    let alpha_repo_path = alpha_worktree.clone();
    let beta_repo_path = beta_worktree.clone();
    let start = Arc::new(Barrier::new(3));
    let alpha_start = Arc::clone(&start);
    let beta_start = Arc::clone(&start);
    let alpha_handle = thread::spawn(move || -> Result<(), String> {
        alpha_start.wait();
        run_build(BuildConfig {
            repo_path: alpha_repo_path,
            output_dir: alpha_knowledge_dir,
            incremental: false,
            ref_name: None,
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
    });
    let beta_handle = thread::spawn(move || -> Result<(), String> {
        beta_start.wait();
        run_build(BuildConfig {
            repo_path: beta_repo_path,
            output_dir: beta_knowledge_dir,
            incremental: false,
            ref_name: None,
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
    });
    start.wait();

    alpha_handle.join().unwrap().map_err(io_error)?;
    beta_handle.join().unwrap().map_err(io_error)?;

    let alpha_read_target = resolve_graph_read_target(Some(&alpha_worktree), None)?;
    let beta_read_target = resolve_graph_read_target(Some(&beta_worktree), None)?;
    let default_ref = RefName::new("main")?;
    let store = GraphObjectStore::new(knowledge_dir.join("graph"));

    assert!(store.ref_path(&alpha_read_target.requested).is_file());
    assert!(store.ref_path(&beta_read_target.requested).is_file());

    let alpha_graph = store.read_graph(
        &alpha_read_target.requested,
        alpha_read_target.fallback.as_ref(),
        Some(&default_ref),
    )?;
    let beta_graph = store.read_graph(
        &beta_read_target.requested,
        beta_read_target.fallback.as_ref(),
        Some(&default_ref),
    )?;
    assert!(
        alpha_graph
            .leaves
            .iter()
            .any(|leaf| leaf.base.name == "alpha_graph")
    );
    assert!(
        beta_graph
            .leaves
            .iter()
            .any(|leaf| leaf.base.name == "beta_graph")
    );
    Ok(())
}

#[test]
fn ensure_fresh_waits_for_requested_branch_ref_before_returning()
-> Result<(), Box<dyn std::error::Error>> {
    let repo = init_repo()?;
    let knowledge_dir = repo.path().join(".orbit/knowledge");
    run_build(BuildConfig {
        repo_path: repo.path().to_path_buf(),
        output_dir: knowledge_dir.clone(),
        incremental: false,
        ref_name: None,
    })?;

    git(repo.path(), &["branch", "feature/wait"])?;
    let feature_worktree = repo.path().join("wt-wait");
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            feature_worktree.to_str().unwrap(),
            "feature/wait",
        ],
    )?;
    write_rust_source(&feature_worktree, "feature_graph")?;
    commit_all_with_date(&feature_worktree, "feature graph", "2030-01-04T00:00:00Z")?;

    let lock_path = knowledge_dir.join("refresh.lock");
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    lock_file.lock_exclusive()?;

    let (status_tx, status_rx) = mpsc::channel();
    let wait_knowledge_dir = knowledge_dir.clone();
    let wait_repo_path = feature_worktree.clone();
    let waiter = thread::spawn(move || {
        let status = ensure_fresh(&wait_knowledge_dir, &wait_repo_path).map_err(|e| e.to_string());
        status_tx.send(status).unwrap();
    });

    assert!(matches!(
        status_rx.recv_timeout(Duration::from_millis(300)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));

    lock_file.unlock()?;
    run_build(BuildConfig {
        repo_path: feature_worktree.clone(),
        output_dir: knowledge_dir.clone(),
        incremental: true,
        ref_name: None,
    })?;

    let status = status_rx
        .recv_timeout(Duration::from_secs(10))
        .map_err(|error| io_error(error.to_string()))?
        .map_err(io_error)?;
    waiter.join().unwrap();
    assert_eq!(status, RefreshStatus::SkippedConcurrent);

    let read_target = resolve_graph_read_target(Some(&feature_worktree), None)?;
    let graph_store = GraphObjectStore::new(knowledge_dir.join("graph"));
    let graph = graph_store.read_graph(
        &read_target.requested,
        read_target.fallback.as_ref(),
        read_target.default.as_ref(),
    )?;
    assert!(
        graph
            .leaves
            .iter()
            .any(|leaf| leaf.base.name == "feature_graph")
    );
    Ok(())
}

#[test]
fn pack_regression_selector_opens_from_branch_ref_layout() -> Result<(), Box<dyn std::error::Error>>
{
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()?;
    let temp = TempDir::new()?;
    let knowledge_dir = temp.path().join("knowledge");
    let build_ref = RefName::new("pack-smoke")?;

    run_build(BuildConfig {
        repo_path: repo_root.clone(),
        output_dir: knowledge_dir.clone(),
        incremental: false,
        ref_name: Some(build_ref.clone()),
    })?;

    let store = KnowledgeStore::open(&knowledge_dir, &build_ref, None, None)?;
    let selector: Selector =
        "symbol:crates/orbit-cli/src/command/graph.rs#GraphSearchArgs::execute:method".parse()?;
    let pack = store.pack(&[selector])?;

    assert_eq!(pack.total_nodes, 1);
    assert!(pack.unresolved_selectors.is_empty());
    assert_eq!(
        pack.entries[0].selector,
        "symbol:crates/orbit-cli/src/command/graph.rs#GraphSearchArgs::execute:method"
    );
    Ok(())
}

fn init_repo() -> Result<TempDir, Box<dyn std::error::Error>> {
    init_repo_with_default("main")
}

fn init_repo_with_default(default_branch: &str) -> Result<TempDir, Box<dyn std::error::Error>> {
    let repo = TempDir::new()?;
    git(repo.path(), &["init", "-b", default_branch])?;
    git(repo.path(), &["config", "user.name", "Orbit Tests"])?;
    git(
        repo.path(),
        &["config", "user.email", "orbit-tests@example.com"],
    )?;
    fs::write(repo.path().join("README.md"), "hello\n")?;
    write_rust_source(repo.path(), "main_graph")?;
    commit_all(repo.path(), "initial commit")?;
    Ok(repo)
}

fn write_rust_source(repo: &Path, function_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(repo.join("src"))?;
    fs::write(
        repo.join("src/lib.rs"),
        format!("pub fn {function_name}() -> &'static str {{\n    \"{function_name}\"\n}}\n"),
    )?;
    Ok(())
}

fn commit_all(repo: &Path, message: &str) -> Result<(), Box<dyn std::error::Error>> {
    git(repo, &["add", "-A"])?;
    git(repo, &["commit", "-m", message])?;
    Ok(())
}

fn commit_all_with_date(
    repo: &Path,
    message: &str,
    timestamp: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    git(repo, &["add", "-A"])?;
    git_with_env(
        repo,
        &["commit", "-m", message],
        &[
            ("GIT_AUTHOR_DATE", timestamp),
            ("GIT_COMMITTER_DATE", timestamp),
        ],
    )?;
    Ok(())
}

fn sample_graph(function_name: &str) -> CodebaseGraphV1 {
    let root_id = "dir:.".to_string();
    let file_id = "file:src/lib.rs".to_string();
    let leaf_id = format!("symbol:src/lib.rs#{function_name}:function");

    CodebaseGraphV1 {
        root_dir_id: root_id.clone(),
        dirs: vec![DirNode {
            base: base_node(&root_id, ".", ".", None),
            dir_children: Vec::new(),
            file_children: vec![file_id.clone()],
        }],
        files: vec![FileNode {
            base: base_node(&file_id, "lib.rs", "src/lib.rs", Some(&root_id)),
            extension: Some("rs".to_string()),
            source_blob_hash: None,
            imports: Vec::new(),
            exports: Vec::new(),
            re_exports: Vec::new(),
            leaf_children: vec![leaf_id.clone()],
        }],
        leaves: vec![LeafNode {
            base: base_node(
                &leaf_id,
                function_name,
                &format!("src/lib.rs#{function_name}"),
                Some(&file_id),
            ),
            kind: LeafKind::Function,
            source: format!("fn {function_name}() {{}}\n"),
            source_blob_hash: None,
            source_hash: None,
            file_hash_at_capture: None,
            history: Vec::new(),
            input_signature: Vec::new(),
            output_signature: Vec::new(),
            start_line: Some(1),
            end_line: Some(1),
            children: Vec::new(),
        }],
    }
}

fn base_node(id: &str, name: &str, location: &str, parent_id: Option<&str>) -> BaseNodeFields {
    BaseNodeFields {
        id: id.to_string(),
        identity_key: id.to_string(),
        object_hash: None,
        name: name.to_string(),
        location: location.to_string(),
        language: "rust".to_string(),
        description: String::new(),
        parent_id: parent_id.map(ToOwned::to_owned),
        is_locked: false,
        lineage_locked: false,
        lock_owner: None,
        lock_reason: String::new(),
        task_ids: Vec::new(),
        structural_conflict: false,
    }
}

fn write_manifest(knowledge_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(knowledge_dir)?;
    fs::write(
        knowledge_dir.join("manifest.json"),
        "{\n  \"generated_at\": \"2026-04-20T00:00:00Z\"\n}\n",
    )?;
    Ok(())
}

fn write_legacy_current_ref(
    knowledge_dir: &Path,
    current_ref: &orbit_knowledge::graph::object_store::CurrentRef,
) -> Result<(), Box<dyn std::error::Error>> {
    let legacy_path = knowledge_dir.join("graph/refs/current.json");
    if let Some(parent) = legacy_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        legacy_path,
        format!("{}\n", serde_json::to_string_pretty(current_ref)?),
    )?;
    Ok(())
}

fn git(repo: &Path, args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    git_with_env(repo, args, &[])
}

fn git_with_env(
    repo: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
) -> Result<String, Box<dyn std::error::Error>> {
    let mut command = Command::new("git");
    command.args(args).current_dir(repo);
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed in '{}': {}",
            args.join(" "),
            repo.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn io_error(message: String) -> std::io::Error {
    std::io::Error::other(message)
}
