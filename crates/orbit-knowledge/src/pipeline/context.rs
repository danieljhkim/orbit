use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::graph::nodes::CodebaseGraphV1;
use crate::graph::object_store::RefName;
use crate::pipeline::GitCheckoutIdentity;

/// Configuration for a build run.
pub struct BuildConfig {
    pub repo_path: PathBuf,
    pub output_dir: PathBuf,
    pub incremental: bool,
    pub ref_name: Option<RefName>,
}

/// Mutable state passed through the pipeline stages.
pub struct PipelineContext {
    pub repo_path: PathBuf,
    pub output_dir: PathBuf,
    pub incremental: bool,
    pub ref_name: RefName,
    pub default_ref_name: Option<RefName>,
    pub(crate) checkout_identity: Option<GitCheckoutIdentity>,
    /// Relative file paths discovered by scan.
    pub file_paths: Vec<PathBuf>,
    /// SHA-256 hashes keyed by relative path string.
    pub new_hashes: HashMap<String, String>,
    /// Paths that changed since last build (incremental mode).
    pub changed_paths: Vec<String>,
    /// The assembled graph.
    pub graph: CodebaseGraphV1,
}

impl PipelineContext {
    pub fn new(config: BuildConfig, ref_name: RefName, default_ref_name: Option<RefName>) -> Self {
        Self {
            repo_path: config.repo_path,
            output_dir: config.output_dir,
            incremental: config.incremental,
            ref_name,
            default_ref_name,
            checkout_identity: None,
            file_paths: Vec::new(),
            new_hashes: HashMap::new(),
            changed_paths: Vec::new(),
            graph: CodebaseGraphV1 {
                root_dir_id: String::new(),
                dirs: Vec::new(),
                files: Vec::new(),
                leaves: Vec::new(),
            },
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
