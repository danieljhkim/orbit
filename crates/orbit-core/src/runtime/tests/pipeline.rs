//! Sibling tests for `pipeline.rs` (migrated per ORB-00246 / docs/design-patterns/test_layout.md).

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::OrbitRuntime;
use orbit_common::types::Role;
use orbit_tools::ToolContext;

use orbit_common::types::{TaskPriority, TaskStatus, TaskType};
use orbit_store::TaskCreateParams;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn run_tool_context_allowlist_honors_task_wildcard() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let task = runtime
        .stores()
        .tasks()
        .create(TaskCreateParams {
            actor: "test".to_string(),
            parent_id: None,
            title: "Wildcard task".to_string(),
            description: "Exercise wildcard runtime allowlist".to_string(),
            acceptance_criteria: Vec::new(),
            dependencies: Vec::new(),
            relations: Vec::new(),
            tags: Vec::new(),
            plan: String::new(),
            execution_summary: String::new(),
            context_files: Vec::new(),
            workspace_path: Some(runtime.paths().repo_root.to_string_lossy().into_owned()),
            repo_root: None,
            created_by: Some("test".to_string()),
            planned_by: None,
            implemented_by: None,
            status: TaskStatus::Backlog,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Chore,
            external_refs: Vec::new(),
            source_task_id: None,
            crew: None,
            comments: Vec::new(),
        })
        .expect("create task");

    let output = runtime
        .run_tool_with_context_and_role(
            "orbit.task.show",
            json!({ "id": task.id.clone() }),
            Role::Admin,
            ToolContext {
                allowed_tools: vec!["orbit.task.*".to_string()],
                orbit_host: Some(crate::runtime::build_orbit_tool_host(
                    &runtime,
                    Some(task.id.clone()),
                    None,
                )),
                ..Default::default()
            },
        )
        .expect("wildcard activity context should permit orbit.task.show");

    assert_eq!(output["id"], task.id);
}

#[test]
fn graph_tool_refresh_from_linked_worktree_attributes_to_worktree_branch() {
    let fixture = GitWorktreeFixture::new();
    let runtime =
        OrbitRuntime::from_roots(&fixture.global_root, &fixture.main_orbit).expect("build runtime");

    runtime
        .run_tool_with_context_and_role(
            "orbit.graph.pack",
            json!({
                "selectors": ["file:Cargo.toml"],
                "refresh": true,
            }),
            Role::Admin,
            ToolContext {
                cwd: Some(fixture.worktree.to_string_lossy().into_owned()),
                ..Default::default()
            },
        )
        .expect("pack from worktree");

    assert!(
        fixture
            .main_orbit
            .join("knowledge/graph/refs/heads/orbit/ORB-00099-test.json")
            .is_file(),
        "worktree branch ref should be written under shared knowledge dir"
    );
    assert_eq!(
        manifest_head_oid(&fixture.main_orbit.join("knowledge/manifest.json")),
        git_output(&fixture.worktree, &["rev-parse", "HEAD"])
    );
}

struct GitWorktreeFixture {
    _root: tempfile::TempDir,
    global_root: PathBuf,
    main_orbit: PathBuf,
    worktree: PathBuf,
}

impl GitWorktreeFixture {
    fn new() -> Self {
        let root = tempdir().expect("create tempdir");
        let global_root = root.path().join("global");
        let main_repo = root.path().join("repo");
        let worktree = main_repo.join(".orbit/state/worktrees/orb-00099-test");
        std::fs::create_dir_all(main_repo.join("src")).expect("create src dir");
        std::fs::create_dir_all(&global_root).expect("create global root");

        run_git(
            root.path(),
            &["init", main_repo.to_str().expect("main repo path")],
        );
        run_git(&main_repo, &["config", "user.email", "test@example.com"]);
        run_git(&main_repo, &["config", "user.name", "Test User"]);
        std::fs::write(
            main_repo.join("Cargo.toml"),
            "[package]\nname = \"orb_00099_fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\npath = \"src/lib.rs\"\n",
        )
        .expect("write manifest");
        std::fs::write(main_repo.join("src/lib.rs"), "pub fn main_branch() {}\n")
            .expect("write lib");
        run_git(&main_repo, &["add", "Cargo.toml", "src/lib.rs"]);
        run_git(&main_repo, &["commit", "-m", "initial"]);
        run_git(&main_repo, &["branch", "-M", "agent-main"]);

        let main_orbit = main_repo.join(".orbit");
        std::fs::create_dir_all(&main_orbit).expect("create main orbit dir");
        run_git(
            &main_repo,
            &[
                "worktree",
                "add",
                "-b",
                "orbit/ORB-00099-test",
                worktree.to_str().expect("worktree path"),
            ],
        );
        std::fs::write(worktree.join("src/lib.rs"), "pub fn worktree_branch() {}\n")
            .expect("write worktree lib");
        run_git(&worktree, &["add", "src/lib.rs"]);
        run_git(&worktree, &["commit", "-m", "worktree change"]);

        Self {
            _root: root,
            global_root,
            main_orbit,
            worktree,
        }
    }
}

fn run_git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git stdout is utf8")
        .trim()
        .to_string()
}

fn manifest_head_oid(path: &Path) -> String {
    let raw = std::fs::read_to_string(path).expect("read manifest");
    let manifest: serde_json::Value = serde_json::from_str(&raw).expect("parse manifest");
    manifest["git_head_oid"]
        .as_str()
        .expect("manifest git_head_oid")
        .to_string()
}
