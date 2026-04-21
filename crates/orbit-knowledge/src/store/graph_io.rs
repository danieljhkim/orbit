use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

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
        .join(hash_prefix(object_hash, "object")?)
        .join(format!("{object_hash}.json"));
    let value: Value = read_json_file(&path).map_err(|error| {
        KnowledgeError::knowledge_unavailable(format!(
            "graph object `{object_hash}` is unavailable at {}: {error}",
            path.display()
        ))
    })?;
    let actual_hash = sha256_hex(canonical_json(&value).as_bytes());
    if actual_hash != object_hash {
        return Err(KnowledgeError::invalid_data(format!(
            "object hash mismatch for {}: expected `{object_hash}`, got `{actual_hash}`",
            path.display()
        )));
    }
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
        .join(hash_prefix(blob_hash, "blob")?)
        .join(format!("{blob_hash}.txt"));
    let source = fs::read_to_string(&path).map_err(|error| {
        KnowledgeError::knowledge_unavailable(format!(
            "graph blob `{blob_hash}` is unavailable at {}: {error}",
            path.display()
        ))
    })?;
    let actual_hash = sha256_hex(source.as_bytes());
    if actual_hash != blob_hash {
        return Err(KnowledgeError::invalid_data(format!(
            "blob hash mismatch for {}: expected `{blob_hash}`, got `{actual_hash}`",
            path.display()
        )));
    }
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

fn canonical_json(value: &Value) -> String {
    let sorted = sort_json_value(value.clone());
    serde_json::to_string(&sorted).unwrap()
}

fn hash_prefix<'a>(hash: &'a str, label: &str) -> Result<&'a str, KnowledgeError> {
    if hash.len() < 2 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(KnowledgeError::invalid_data(format!(
            "invalid {label} hash `{hash}`"
        )));
    }
    Ok(&hash[..2])
}

fn sort_json_value(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<String, Value> = map
                .into_iter()
                .map(|(key, value)| (key, sort_json_value(value)))
                .collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_json_value).collect()),
        other => other,
    }
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}
