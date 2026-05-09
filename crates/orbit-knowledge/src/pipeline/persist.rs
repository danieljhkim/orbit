use std::fs;

use chrono::Utc;
use serde_json::json;

use crate::error::KnowledgeError;
use crate::graph::object_store::GraphObjectStore;
use crate::io::write_text_atomic_durable;
use crate::pipeline::context::PipelineContext;

/// Write the assembled graph to the content-addressed object store.
pub fn persist_graph(ctx: &PipelineContext) -> Result<String, KnowledgeError> {
    let store = GraphObjectStore::new(ctx.graph_dir());
    store.prepare_refs_layout(ctx.default_ref_name.as_ref())?;

    let mut current_ref = store.write_graph(&ctx.graph)?;
    if let Some(identity) = ctx.checkout_identity.as_ref() {
        current_ref.git_head_oid = Some(identity.head_oid.clone());
        current_ref.git_tree_oid = Some(identity.tree_oid.clone());
    }
    store.write_ref_atomic(&ctx.ref_name, &current_ref)?;

    Ok(current_ref.root_graph_hash)
}

/// Write a `manifest.json` with build metadata.
pub fn write_manifest(ctx: &PipelineContext) -> Result<(), KnowledgeError> {
    fs::create_dir_all(&ctx.output_dir)
        .map_err(|e| KnowledgeError::io(format!("mkdir {}: {e}", ctx.output_dir.display())))?;

    let mut manifest = json!({
        "generated_at": Utc::now().to_rfc3339(),
        "graph": "graph/",
        "file_count": ctx.graph.files.len(),
        "leaf_count": ctx.graph.leaves.len(),
    });
    if let Some(identity) = ctx.checkout_identity.as_ref()
        && let Some(manifest) = manifest.as_object_mut()
    {
        manifest.insert("git_head_oid".to_string(), json!(identity.head_oid.clone()));
        manifest.insert("git_tree_oid".to_string(), json!(identity.tree_oid.clone()));
    }

    let json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| KnowledgeError::invalid_data(format!("manifest serialize: {e}")))?;
    write_text_atomic_durable(&ctx.manifest_path(), &format!("{json}\n"))
        .map_err(|e| KnowledgeError::io(format!("write manifest: {e}")))?;
    Ok(())
}
