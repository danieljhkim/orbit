use std::path::PathBuf;

use clap::{Args, Subcommand};
use orbit_common::types::TaskArtifact;
use orbit_core::command::task::TaskUpdateParams;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

use super::output::task_to_json_for_runtime;

#[derive(Args)]
#[command(about = "Manage task artifact files")]
pub struct TaskArtifactCommand {
    #[command(subcommand)]
    pub command: TaskArtifactSubcommand,
}

impl Execute for TaskArtifactCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum TaskArtifactSubcommand {
    /// Store a UTF-8 source file under a task's artifacts directory
    Put(TaskArtifactPutArgs),
}

impl Execute for TaskArtifactSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            TaskArtifactSubcommand::Put(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct TaskArtifactPutArgs {
    /// Task ID
    pub id: String,
    /// UTF-8 source file to store as a task artifact
    pub source_path: PathBuf,
    /// Artifact path relative to the task artifacts directory. Defaults to the source file name.
    #[arg(long = "path")]
    pub artifact_path: Option<String>,
    /// Explicit agent name to persist on the task artifact update
    #[arg(long)]
    pub agent: Option<String>,
    /// Explicit agent model to persist on the task artifact update
    #[arg(long)]
    pub model: Option<String>,
    /// Output the updated task as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskArtifactPutArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let TaskArtifactPutArgs {
            id,
            source_path,
            artifact_path,
            agent,
            model,
            json,
        } = self;
        let artifact = TaskArtifact::from_utf8_source_file(&source_path, artifact_path.as_deref())?;
        let artifact_path = artifact.path.clone();
        let task = runtime.update_task_with_identity(
            &id,
            TaskUpdateParams {
                upsert_artifacts: vec![artifact],
                ..Default::default()
            },
            agent,
            model,
        )?;

        if json {
            crate::output::json::print_pretty(&task_to_json_for_runtime(runtime, &task)?)
        } else {
            println!("Stored artifact '{artifact_path}' on task '{}'", task.id);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clap::Parser;
    use orbit_core::OrbitRuntime;
    use orbit_core::command::task::TaskAddParams;
    use tempfile::tempdir;

    use crate::command::Execute;
    use crate::command::{Cli, Commands};

    use super::{TaskArtifactCommand, TaskArtifactPutArgs, TaskArtifactSubcommand};
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
        assert_eq!(artifacts[0].content, "stored\n");
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
}
