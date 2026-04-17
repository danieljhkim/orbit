//! Build pipeline: scan → hash → build graph → persist.
//!
//! Each stage is a plain function operating on a shared [`PipelineContext`].

pub mod build;
pub mod context;
pub mod hash;
pub mod persist;
pub mod scan;

use std::path::Path;
use std::process::Command;

use crate::error::KnowledgeError;
use context::{BuildConfig, PipelineContext};

/// Run the full build pipeline.
///
/// Scans the repo, computes hashes, builds the graph (dirs, files, leaves),
/// persists the graph to the content-addressed store, and writes the manifest.
pub fn run_build(config: BuildConfig) -> Result<PipelineContext, KnowledgeError> {
    let mut ctx = PipelineContext::new(config);

    scan::scan_repo(&mut ctx)?;
    hash::compute_hashes(&mut ctx)?;
    hash::detect_changes(&mut ctx)?;

    build::build_graph_dirs(&mut ctx)?;
    build::build_graph_files(&mut ctx)?;
    build::build_graph_leaves(&mut ctx)?;

    persist::persist_graph(&ctx)?;
    persist::write_manifest(&ctx)?;
    hash::save_hash_cache(&ctx)?;

    Ok(ctx)
}

/// Ensure the knowledge graph is up-to-date with the current checkout.
///
/// Rebuilds when the checkout is dirty, or when the manifest's `generated_at`
/// timestamp lags behind `git log -1 --format=%cI`. If the manifest is missing,
/// runs an incremental build as the first refresh.
/// Returns `true` if a rebuild was triggered, `false` if already fresh.
pub fn ensure_fresh(knowledge_dir: &Path, repo_path: &Path) -> Result<bool, KnowledgeError> {
    let manifest_path = knowledge_dir.join("manifest.json");
    let worktree_dirty = git_worktree_dirty(repo_path).unwrap_or(false);
    let head_ts = git_head_timestamp(repo_path);

    let needs_rebuild = if worktree_dirty {
        true
    } else if manifest_path.is_file() {
        let raw = std::fs::read_to_string(&manifest_path)
            .map_err(|e| KnowledgeError::knowledge_unavailable(format!("read manifest: {e}")))?;
        let manifest: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| KnowledgeError::knowledge_unavailable(format!("parse manifest: {e}")))?;
        let generated_at = manifest
            .get("generated_at")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
        match (generated_at, head_ts) {
            (Some(generated), Some(head_ts)) => head_ts > generated,
            (None, _) => true,  // corrupt or missing timestamp
            (_, None) => false, // not a git repo or git unavailable
        }
    } else {
        true // no manifest → first build
    };

    if !needs_rebuild {
        return Ok(false);
    }

    let config = BuildConfig {
        repo_path: repo_path.to_path_buf(),
        output_dir: knowledge_dir.to_path_buf(),
        incremental: manifest_path.is_file(), // incremental if manifest exists
    };
    run_build(config)
        .map_err(|e| KnowledgeError::knowledge_unavailable(format!("auto-refresh failed: {e}")))?;

    Ok(true)
}

/// Get the committer timestamp of HEAD as a fixed-offset DateTime.
fn git_head_timestamp(repo_path: &Path) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    let output = Command::new("git")
        .args(["log", "-1", "--format=%cI"])
        .current_dir(repo_path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let ts_str = String::from_utf8_lossy(&output.stdout);
    chrono::DateTime::parse_from_rfc3339(ts_str.trim()).ok()
}

fn git_worktree_dirty(repo_path: &Path) -> Option<bool> {
    let output = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .current_dir(repo_path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(!output.stdout.is_empty())
}
