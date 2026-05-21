use std::collections::{BTreeMap, HashMap};
use std::fs;

use rayon::prelude::*;
use sha2::{Digest, Sha256};

use crate::error::KnowledgeError;
use crate::pipeline::context::PipelineContext;

/// Compute SHA-256 hashes for all scanned files, populating `ctx.new_hashes`.
pub fn compute_hashes(ctx: &mut PipelineContext) -> Result<(), KnowledgeError> {
    ctx.new_hashes = ctx
        .file_paths
        .par_iter()
        .filter_map(|rel_path| {
            let abs = ctx.repo_path.join(rel_path);
            let bytes = fs::read(&abs).ok()?;
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            Some((
                rel_path.to_string_lossy().into_owned(),
                format!("{:x}", hasher.finalize()),
            ))
        })
        .collect();
    Ok(())
}

/// Compare new hashes against the cached hashes, populating `ctx.changed_paths`.
///
/// In non-incremental mode, all paths are treated as changed.
pub fn detect_changes(ctx: &mut PipelineContext) -> Result<(), KnowledgeError> {
    if !ctx.incremental {
        ctx.changed_paths = hashed_file_paths(ctx);
        return Ok(());
    }

    let old_hashes = load_hash_cache(ctx);
    ctx.changed_paths = ctx
        .file_paths
        .iter()
        .filter_map(|rel_path| {
            let path = rel_path.to_string_lossy().into_owned();
            let hash = ctx.new_hashes.get(&path)?;
            (old_hashes.get(&path) != Some(hash)).then_some(path)
        })
        .collect();

    Ok(())
}

/// Persist current hashes to `output_dir/hashes.json`.
pub fn save_hash_cache(ctx: &PipelineContext) -> Result<(), KnowledgeError> {
    fs::create_dir_all(&ctx.output_dir)
        .map_err(|e| KnowledgeError::io(format!("mkdir {}: {e}", ctx.output_dir.display())))?;
    let sorted_hashes: BTreeMap<&String, &String> = ctx.new_hashes.iter().collect();
    let json = serde_json::to_string_pretty(&sorted_hashes)
        .map_err(|e| KnowledgeError::invalid_data(format!("hash cache serialize: {e}")))?;
    fs::write(ctx.hashes_path(), json)
        .map_err(|e| KnowledgeError::io(format!("write hashes: {e}")))?;
    Ok(())
}

fn hashed_file_paths(ctx: &PipelineContext) -> Vec<String> {
    ctx.file_paths
        .iter()
        .filter_map(|rel_path| {
            let path = rel_path.to_string_lossy().into_owned();
            ctx.new_hashes.contains_key(&path).then_some(path)
        })
        .collect()
}

fn load_hash_cache(ctx: &PipelineContext) -> HashMap<String, String> {
    let path = ctx.hashes_path();
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
