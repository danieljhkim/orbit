use std::path::PathBuf;

use clap::Parser;
use orbit_core::OrbitRuntime;
use orbit_core::command::task::TaskAddParams;
use tempfile::tempdir;

use crate::command::Execute;
use crate::command::{Cli, Commands};

use super::super::artifact::{TaskArtifactCommand, TaskArtifactPutArgs, TaskArtifactSubcommand};
use crate::command::task::TaskSubcommand;

#[test]
fn cli_parses_task_artifact_put() {
    let cli = Cli::parse_from([
        "orbit",
        "task",
        "artifact",
        "put",
        "T1",
        "./summary.md",
        "--path",
        "reports/summary.md",
        "--json",
    ]);

    let Commands::Task(task_command) = cli.command else {
        panic!("expected task command");
    };
    let TaskSubcommand::Artifact(TaskArtifactCommand {
        command: TaskArtifactSubcommand::Put(args),
    }) = task_command.command
    else {
        panic!("expected task artifact put command");
    };

    assert_eq!(args.id, "T1");
    assert_eq!(args.source_path, PathBuf::from("./summary.md"));
    assert_eq!(args.artifact_path.as_deref(), Some("reports/summary.md"));
    assert!(args.json);
}

#[test]
fn artifact_put_writes_to_task_artifact_store() {
    let (_root, runtime, repo_root) = test_runtime();
    let source = repo_root.join("summary.md");
    std::fs::write(&source, "stored\n").expect("write source");
    let task = runtime
        .add_task(TaskAddParams {
            title: "Artifact store".to_string(),
            description: "Store a task artifact".to_string(),
            workspace_path: Some(repo_root.to_string_lossy().into_owned()),
            ..Default::default()
        })
        .expect("create task");

    TaskArtifactPutArgs {
        id: task.id.clone(),
        source_path: source,
        artifact_path: Some("reports/summary.md".to_string()),
        agent: Some("codex".to_string()),
        model: Some("gpt-5".to_string()),
        json: false,
    }
    .execute(&runtime)
    .expect("put artifact");

    let artifacts = runtime
        .get_task_artifacts(&task.id)
        .expect("read task artifacts");
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].path, "reports/summary.md");
    assert_eq!(artifacts[0].text_content(), Some("stored\n"));
}

fn test_runtime() -> (tempfile::TempDir, OrbitRuntime, PathBuf) {
    let root = tempdir().expect("create tempdir");
    let global_root = root.path().join("global");
    let repo_root = root.path().join("repo");
    let workspace_root = repo_root.join(".orbit");
    std::fs::create_dir_all(&global_root).expect("create global root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
    (root, runtime, repo_root)
}
