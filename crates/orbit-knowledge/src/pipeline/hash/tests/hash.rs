use std::path::PathBuf;

use tempfile::TempDir;

use super::super::*;

use crate::graph::object_store::RefName;
use crate::pipeline::context::BuildConfig;

#[test]
fn compute_hashes_skips_missing_files_without_error() {
    let repo = TempDir::new().expect("create repo tempdir");
    fs::write(repo.path().join("readable.rs"), b"pub fn readable() {}\n")
        .expect("write readable file");

    let mut ctx = test_context(&repo);
    ctx.file_paths = vec![PathBuf::from("missing.rs"), PathBuf::from("readable.rs")];

    compute_hashes(&mut ctx).expect("hash computation succeeds");

    assert!(ctx.new_hashes.contains_key("readable.rs"));
    assert!(!ctx.new_hashes.contains_key("missing.rs"));
}

fn test_context(repo: &TempDir) -> PipelineContext {
    let config = BuildConfig {
        repo_path: repo.path().to_path_buf(),
        output_dir: repo.path().join(".knowledge"),
        incremental: false,
        ref_name: Some(RefName::new("main").expect("valid ref name")),
    };
    PipelineContext::new(config, RefName::new("main").expect("valid ref name"), None)
}
