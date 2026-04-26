use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::graph::nodes::CodebaseGraphV1;
use crate::graph::object_store::RefName;
use crate::task_id_pattern::TaskIdPattern;

/// Configuration for a build run.
pub struct BuildConfig {
    pub repo_path: PathBuf,
    pub output_dir: PathBuf,
    pub incremental: bool,
    pub ref_name: Option<RefName>,
    /// Task-ID extraction pattern. `None` means "use the Orbit default".
    pub task_id_pattern: Option<TaskIdPattern>,
}

/// Mutable state passed through the pipeline stages.
pub struct PipelineContext {
    pub repo_path: PathBuf,
    pub output_dir: PathBuf,
    pub incremental: bool,
    pub ref_name: RefName,
    pub default_ref_name: Option<RefName>,
    /// Relative file paths discovered by scan.
    pub file_paths: Vec<PathBuf>,
    /// SHA-256 hashes keyed by relative path string.
    pub new_hashes: HashMap<String, String>,
    /// Paths that changed since last build (incremental mode).
    pub changed_paths: Vec<String>,
    /// The assembled graph.
    pub graph: CodebaseGraphV1,
    /// Resolved task-ID extraction pattern for this build (default Orbit when
    /// the build config did not supply one).
    pub task_id_pattern: TaskIdPattern,
}

impl PipelineContext {
    pub fn new(config: BuildConfig, ref_name: RefName, default_ref_name: Option<RefName>) -> Self {
        let task_id_pattern = config.task_id_pattern.unwrap_or_default();
        Self {
            repo_path: config.repo_path,
            output_dir: config.output_dir,
            incremental: config.incremental,
            ref_name,
            default_ref_name,
            file_paths: Vec::new(),
            new_hashes: HashMap::new(),
            changed_paths: Vec::new(),
            graph: CodebaseGraphV1 {
                root_dir_id: String::new(),
                dirs: Vec::new(),
                files: Vec::new(),
                leaves: Vec::new(),
            },
            task_id_pattern,
        }
    }

    pub fn graph_dir(&self) -> PathBuf {
        self.output_dir.join("graph")
    }

    pub fn hashes_path(&self) -> PathBuf {
        self.output_dir.join("hashes.json")
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.output_dir.join("manifest.json")
    }

    /// Resolve a relative path against the repo root.
    pub fn abs_path(&self, rel: &Path) -> PathBuf {
        self.repo_path.join(rel)
    }
}
