use std::collections::HashMap;
use std::fs;

use sha2::{Digest, Sha256};

use crate::error::KnowledgeError;
use crate::pipeline::context::PipelineContext;

/// Compute SHA-256 hashes for all scanned files, populating `ctx.new_hashes`.
pub fn compute_hashes(ctx: &mut PipelineContext) -> Result<(), KnowledgeError> {
    for rel_path in &ctx.file_paths {
        let abs = ctx.repo_path.join(rel_path);
        match fs::read(&abs) {
            Ok(bytes) => {
                let mut hasher = Sha256::new();
                hasher.update(&bytes);
                let hash = format!("{:x}", hasher.finalize());
                ctx.new_hashes
                    .insert(rel_path.to_string_lossy().into_owned(), hash);
            }
            Err(_) => {
                // Skip unreadable files
            }
        }
    }
    Ok(())
}

/// Compare new hashes against the cached hashes, populating `ctx.changed_paths`.
///
/// In non-incremental mode, all paths are treated as changed.
pub fn detect_changes(ctx: &mut PipelineContext) -> Result<(), KnowledgeError> {
    if !ctx.incremental {
        ctx.changed_paths = ctx.new_hashes.keys().cloned().collect();
        return Ok(());
    }

    let old_hashes = load_hash_cache(ctx);
    ctx.changed_paths = ctx
        .new_hashes
        .iter()
        .filter(|(path, hash)| old_hashes.get(*path) != Some(*hash))
        .map(|(path, _)| path.clone())
        .collect();

    Ok(())
}

/// Persist current hashes to `output_dir/hashes.json`.
pub fn save_hash_cache(ctx: &PipelineContext) -> Result<(), KnowledgeError> {
    fs::create_dir_all(&ctx.output_dir)
        .map_err(|e| KnowledgeError::io(format!("mkdir {}: {e}", ctx.output_dir.display())))?;
    let json = serde_json::to_string_pretty(&ctx.new_hashes)
        .map_err(|e| KnowledgeError::invalid_data(format!("hash cache serialize: {e}")))?;
    fs::write(ctx.hashes_path(), json)
        .map_err(|e| KnowledgeError::io(format!("write hashes: {e}")))?;
    Ok(())
}

fn load_hash_cache(ctx: &PipelineContext) -> HashMap<String, String> {
    let path = ctx.hashes_path();
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}
