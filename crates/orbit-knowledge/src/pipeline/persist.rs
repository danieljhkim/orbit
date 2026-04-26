use std::fs;

use chrono::Utc;
use serde_json::json;

use crate::error::KnowledgeError;
use crate::graph::object_store::GraphObjectStore;
use crate::io::write_text_atomic_durable;
use crate::pipeline::attribute::AttributeOutcome;
use crate::pipeline::context::PipelineContext;
use crate::store::task_commits;

/// Write the assembled graph to the content-addressed object store, then
/// persist the attribution sidecar and the updated `last_attributed_commit`
/// cursor alongside the ref.
pub fn persist_graph(
    ctx: &PipelineContext,
    outcome: &AttributeOutcome,
) -> Result<String, KnowledgeError> {
    let store = GraphObjectStore::new(ctx.graph_dir());
    store.prepare_refs_layout(ctx.default_ref_name.as_ref())?;

    let mut current_ref = store.write_graph(&ctx.graph)?;
    current_ref.last_attributed_commit = outcome.head_sha.clone();
    store.write_ref_atomic(&ctx.ref_name, &current_ref)?;

    let sidecar_path = task_commits::sidecar_path(&ctx.output_dir, ctx.ref_name.as_str());
    task_commits::save(&sidecar_path, &outcome.sidecar)?;

    Ok(current_ref.root_graph_hash)
}

/// Write a `manifest.json` with build metadata.
pub fn write_manifest(ctx: &PipelineContext) -> Result<(), KnowledgeError> {
    fs::create_dir_all(&ctx.output_dir)
        .map_err(|e| KnowledgeError::io(format!("mkdir {}: {e}", ctx.output_dir.display())))?;

    let manifest = json!({
        "generated_at": Utc::now().to_rfc3339(),
        "graph": "graph/",
        "file_count": ctx.graph.files.len(),
        "leaf_count": ctx.graph.leaves.len(),
        "task_id_pattern": ctx.task_id_pattern.as_str(),
    });

    let json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| KnowledgeError::invalid_data(format!("manifest serialize: {e}")))?;
    write_text_atomic_durable(&ctx.manifest_path(), &format!("{json}\n"))
        .map_err(|e| KnowledgeError::io(format!("write manifest: {e}")))?;
    Ok(())
}
