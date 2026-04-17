use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::error::KnowledgeError;

pub(super) fn read_graph_object(
    knowledge_dir: &Path,
    object_hash: &str,
    cache: &mut HashMap<String, Value>,
) -> Result<Value, KnowledgeError> {
    if let Some(value) = cache.get(object_hash) {
        return Ok(value.clone());
    }

    let path = knowledge_dir
        .join("graph/objects")
        .join(&object_hash[..2])
        .join(format!("{object_hash}.json"));
    let value: Value = read_json_file(&path).map_err(|error| {
        KnowledgeError::knowledge_unavailable(format!(
            "graph object `{object_hash}` is unavailable at {}: {error}",
            path.display()
        ))
    })?;
    cache.insert(object_hash.to_string(), value.clone());
    Ok(value)
}

pub(super) fn extract_leaf_source(
    knowledge_dir: &Path,
    object: &Value,
    blob_cache: &mut HashMap<String, String>,
) -> Result<Option<String>, KnowledgeError> {
    if let Some(source) = object
        .get("node")
        .and_then(|node| node.get("source"))
        .and_then(Value::as_str)
        .filter(|source| !source.is_empty())
    {
        return Ok(Some(source.to_string()));
    }

    let Some(blob_hash) = object
        .get("node")
        .and_then(|node| node.get("source_blob_hash"))
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };

    if let Some(source) = blob_cache.get(blob_hash) {
        return Ok(Some(source.clone()));
    }

    let path = knowledge_dir
        .join("graph/blobs")
        .join(&blob_hash[..2])
        .join(format!("{blob_hash}.txt"));
    let source = fs::read_to_string(&path).map_err(|error| {
        KnowledgeError::knowledge_unavailable(format!(
            "graph blob `{blob_hash}` is unavailable at {}: {error}",
            path.display()
        ))
    })?;
    blob_cache.insert(blob_hash.to_string(), source.clone());
    Ok(Some(source))
}

pub(super) fn read_json_file<T>(path: &Path) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&raw).map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ManifestFile {
    pub(super) generated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct CurrentRefFile {
    pub(super) index: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphIndexFile {
    pub(super) nodes: HashMap<String, GraphIndexEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphIndexEntry {
    pub(super) object_hash: String,
    pub(super) node_type: String,
    pub(super) location: String,
    pub(super) kind: Option<String>,
}
