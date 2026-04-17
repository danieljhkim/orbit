//! Content-addressed graph persistence for `.orbit/knowledge/graph/`.
//!
//! Reads and writes the same on-disk format as Python's `orbit_map/graph/store.py`.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::nodes::{CodebaseGraphV1, DirNode, FileNode, LeafNode};
use crate::error::KnowledgeError;

const GRAPH_STORE_SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Internal deserialization types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CurrentRef {
    index: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct ByIdIndex {
    #[serde(default)]
    schema_version: u32,
    root_dir_id: String,
    #[serde(default)]
    root_graph_hash: String,
    #[serde(default)]
    root_object_hash: String,
    #[serde(default)]
    dirs: Vec<String>,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    leaves: Vec<String>,
    nodes: HashMap<String, IndexEntry>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct IndexEntry {
    object_hash: String,
    node_type: String,
    location: String,
    #[serde(default)]
    identity_key: String,
    kind: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct ObjectEnvelope {
    #[serde(default)]
    schema_version: u32,
    object_type: String,
    node_type: String,
    node: Value,
    #[serde(default)]
    child_object_hashes: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// Public store
// ---------------------------------------------------------------------------

pub struct GraphObjectStore {
    graph_dir: PathBuf,
}

impl GraphObjectStore {
    pub fn new(graph_dir: impl Into<PathBuf>) -> Self {
        Self {
            graph_dir: graph_dir.into(),
        }
    }

    // -- directory helpers --------------------------------------------------

    fn refs_dir(&self) -> PathBuf {
        self.graph_dir.join("refs")
    }
    fn index_dir(&self) -> PathBuf {
        self.graph_dir.join("index")
    }
    fn objects_dir(&self) -> PathBuf {
        self.graph_dir.join("objects")
    }
    fn blobs_dir(&self) -> PathBuf {
        self.graph_dir.join("blobs")
    }
    fn object_path(&self, hash: &str) -> Result<PathBuf, KnowledgeError> {
        let prefix = hash_prefix(hash, "object")?;
        Ok(self.objects_dir().join(prefix).join(format!("{hash}.json")))
    }
    fn blob_path(&self, hash: &str) -> Result<PathBuf, KnowledgeError> {
        let prefix = hash_prefix(hash, "blob")?;
        Ok(self.blobs_dir().join(prefix).join(format!("{hash}.txt")))
    }
    fn resolve_index_path(&self, index_ref: &str) -> Result<PathBuf, KnowledgeError> {
        let index_rel = index_ref.strip_prefix("graph/").unwrap_or(index_ref);
        let rel_path = Path::new(index_rel);
        if rel_path.is_absolute()
            || rel_path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(KnowledgeError::invalid_data(format!(
                "invalid graph index path `{index_ref}`"
            )));
        }
        Ok(self.graph_dir.join(rel_path))
    }

    // -----------------------------------------------------------------------
    // Read path
    // -----------------------------------------------------------------------

    pub fn read_graph(&self) -> Result<CodebaseGraphV1, KnowledgeError> {
        // 1. Read refs/current.json
        let current_ref_path = self.refs_dir().join("current.json");
        let current_ref: CurrentRef = read_json_file(&current_ref_path)?;

        // 2. Resolve index path (strip leading "graph/" prefix if present)
        let index_path = self.resolve_index_path(&current_ref.index)?;
        let index: ByIdIndex = read_json_file(&index_path)?;

        // 3. Read dir nodes
        let mut dirs = Vec::with_capacity(index.dirs.len());
        for node_id in &index.dirs {
            let entry = index.nodes.get(node_id).ok_or_else(|| {
                KnowledgeError::invalid_data(format!("dir node {node_id} missing from index"))
            })?;
            let envelope = self.read_object_envelope(&entry.object_hash)?;
            let mut dir: DirNode = serde_json::from_value(envelope.node)
                .map_err(|e| KnowledgeError::invalid_data(format!("dir node parse: {e}")))?;
            dir.base.object_hash = Some(entry.object_hash.clone());
            dirs.push(dir);
        }

        // 4. Read file nodes
        let mut files = Vec::with_capacity(index.files.len());
        for node_id in &index.files {
            let entry = index.nodes.get(node_id).ok_or_else(|| {
                KnowledgeError::invalid_data(format!("file node {node_id} missing from index"))
            })?;
            let envelope = self.read_object_envelope(&entry.object_hash)?;
            let mut file: FileNode = serde_json::from_value(envelope.node)
                .map_err(|e| KnowledgeError::invalid_data(format!("file node parse: {e}")))?;
            file.base.object_hash = Some(entry.object_hash.clone());
            files.push(file);
        }

        // 5. Read leaf nodes (hydrate source from blobs)
        let mut leaves = Vec::with_capacity(index.leaves.len());
        for node_id in &index.leaves {
            let entry = index.nodes.get(node_id).ok_or_else(|| {
                KnowledgeError::invalid_data(format!("leaf node {node_id} missing from index"))
            })?;
            let envelope = self.read_object_envelope(&entry.object_hash)?;
            let mut leaf: LeafNode = serde_json::from_value(envelope.node)
                .map_err(|e| KnowledgeError::invalid_data(format!("leaf node parse: {e}")))?;
            leaf.base.object_hash = Some(entry.object_hash.clone());

            // Hydrate source from blob if present and source is empty
            if let Some(ref blob_hash) = leaf.source_blob_hash
                && leaf.source.is_empty()
            {
                leaf.source = self.read_blob(blob_hash)?;
            }

            leaves.push(leaf);
        }

        Ok(CodebaseGraphV1 {
            root_dir_id: index.root_dir_id,
            dirs,
            files,
            leaves,
        })
    }

    // -----------------------------------------------------------------------
    // Write path
    // -----------------------------------------------------------------------

    pub fn write_graph(&self, graph: &CodebaseGraphV1) -> Result<String, KnowledgeError> {
        if !graph
            .dirs
            .iter()
            .any(|dir| dir.base.id == graph.root_dir_id)
        {
            return Err(KnowledgeError::invalid_data(format!(
                "graph root `{}` is missing from dir nodes",
                graph.root_dir_id
            )));
        }

        self.ensure_dirs()?;

        let mut object_hashes: HashMap<String, String> = HashMap::new();
        let mut index_nodes: HashMap<String, Value> = HashMap::new();

        // 1. Write leaves (children first)
        for leaf in &graph.leaves {
            let mut node_json = serde_json::to_value(leaf)
                .map_err(|e| KnowledgeError::invalid_data(format!("leaf serialize: {e}")))?;

            // Strip object_hash from node data before hashing
            if let Value::Object(ref mut map) = node_json {
                map.remove("object_hash");
            }

            // Write source to blob, update blob hash, clear source in node JSON
            if !leaf.source.is_empty() {
                let bh = self.write_blob(&leaf.source)?;
                if let Value::Object(ref mut map) = node_json {
                    map.insert("source_blob_hash".to_string(), Value::String(bh));
                    map.insert("source".to_string(), Value::String(String::new()));
                }
            }

            let envelope = serde_json::json!({
                "schema_version": GRAPH_STORE_SCHEMA_VERSION,
                "object_type": "graph_node",
                "node_type": "leaf",
                "node": node_json,
                "child_object_hashes": {}
            });

            let object_hash = self.write_json_object(&envelope)?;
            object_hashes.insert(leaf.base.id.clone(), object_hash.clone());
            index_nodes.insert(
                leaf.base.id.clone(),
                serde_json::json!({
                    "object_hash": object_hash,
                    "node_type": "leaf",
                    "location": leaf.base.location,
                    "identity_key": leaf.base.identity_key,
                    "kind": leaf.kind,
                }),
            );
        }

        // 2. Write files
        for file in &graph.files {
            let mut node_json = serde_json::to_value(file)
                .map_err(|e| KnowledgeError::invalid_data(format!("file serialize: {e}")))?;
            if let Value::Object(ref mut map) = node_json {
                map.remove("object_hash");
            }

            let child_hashes: HashMap<String, String> = file
                .leaf_children
                .iter()
                .map(|cid| {
                    let hash = object_hashes.get(cid).ok_or_else(|| {
                        KnowledgeError::invalid_data(format!(
                            "file `{}` references missing child `{cid}`",
                            file.base.id
                        ))
                    })?;
                    Ok((cid.clone(), hash.clone()))
                })
                .collect::<Result<_, KnowledgeError>>()?;

            let envelope = serde_json::json!({
                "schema_version": GRAPH_STORE_SCHEMA_VERSION,
                "object_type": "graph_node",
                "node_type": "file",
                "node": node_json,
                "child_object_hashes": child_hashes,
            });

            let object_hash = self.write_json_object(&envelope)?;
            object_hashes.insert(file.base.id.clone(), object_hash.clone());
            index_nodes.insert(
                file.base.id.clone(),
                serde_json::json!({
                    "object_hash": object_hash,
                    "node_type": "file",
                    "location": file.base.location,
                    "identity_key": file.base.identity_key,
                    "kind": null,
                }),
            );
        }

        // 3. Write dirs from deepest to shallowest so child object hashes exist
        // before their parent directory envelopes are serialized.
        let mut dirs_in_write_order: Vec<&DirNode> = graph.dirs.iter().collect();
        dirs_in_write_order.sort_by(|a, b| {
            dir_depth(&b.base.location)
                .cmp(&dir_depth(&a.base.location))
                .then_with(|| a.base.location.cmp(&b.base.location))
        });

        for dir in dirs_in_write_order {
            let mut node_json = serde_json::to_value(dir)
                .map_err(|e| KnowledgeError::invalid_data(format!("dir serialize: {e}")))?;
            if let Value::Object(ref mut map) = node_json {
                map.remove("object_hash");
            }

            let mut child_hashes: HashMap<String, String> = HashMap::new();
            for cid in &dir.dir_children {
                let hash = object_hashes.get(cid).ok_or_else(|| {
                    KnowledgeError::invalid_data(format!(
                        "dir `{}` references missing dir child `{cid}`",
                        dir.base.id
                    ))
                })?;
                child_hashes.insert(cid.clone(), hash.clone());
            }
            for cid in &dir.file_children {
                let hash = object_hashes.get(cid).ok_or_else(|| {
                    KnowledgeError::invalid_data(format!(
                        "dir `{}` references missing file child `{cid}`",
                        dir.base.id
                    ))
                })?;
                child_hashes.insert(cid.clone(), hash.clone());
            }

            let envelope = serde_json::json!({
                "schema_version": GRAPH_STORE_SCHEMA_VERSION,
                "object_type": "graph_node",
                "node_type": "dir",
                "node": node_json,
                "child_object_hashes": child_hashes,
            });

            let object_hash = self.write_json_object(&envelope)?;
            object_hashes.insert(dir.base.id.clone(), object_hash.clone());
            index_nodes.insert(
                dir.base.id.clone(),
                serde_json::json!({
                    "object_hash": object_hash,
                    "node_type": "dir",
                    "location": dir.base.location,
                    "identity_key": dir.base.identity_key,
                    "kind": null,
                }),
            );
        }

        // 4. Root object hash
        let root_object_hash = object_hashes
            .get(&graph.root_dir_id)
            .cloned()
            .ok_or_else(|| {
                KnowledgeError::invalid_data(format!(
                    "graph root `{}` did not produce an object hash",
                    graph.root_dir_id
                ))
            })?;

        // 5. Write root graph object
        let root_payload = serde_json::json!({
            "schema_version": GRAPH_STORE_SCHEMA_VERSION,
            "object_type": "codebase_graph",
            "root_dir_id": graph.root_dir_id,
            "root_object_hash": root_object_hash,
            "dirs": graph.dirs.iter().map(|d| &d.base.id).collect::<Vec<_>>(),
            "files": graph.files.iter().map(|f| &f.base.id).collect::<Vec<_>>(),
            "leaves": graph.leaves.iter().map(|l| &l.base.id).collect::<Vec<_>>(),
            "node_count": graph.dirs.len() + graph.files.len() + graph.leaves.len(),
        });
        let root_graph_hash = self.write_json_object(&root_payload)?;

        // 6. Write by-id index
        let by_id_index = serde_json::json!({
            "schema_version": GRAPH_STORE_SCHEMA_VERSION,
            "root_dir_id": graph.root_dir_id,
            "root_graph_hash": root_graph_hash,
            "root_object_hash": root_object_hash,
            "dirs": graph.dirs.iter().map(|d| &d.base.id).collect::<Vec<_>>(),
            "files": graph.files.iter().map(|f| &f.base.id).collect::<Vec<_>>(),
            "leaves": graph.leaves.iter().map(|l| &l.base.id).collect::<Vec<_>>(),
            "nodes": index_nodes,
        });
        write_json_file(&self.index_dir().join("by-id.json"), &by_id_index)?;

        // 7. Write refs/current.json
        let current_ref = serde_json::json!({
            "schema_version": GRAPH_STORE_SCHEMA_VERSION,
            "root_graph_hash": root_graph_hash,
            "root_object_hash": root_object_hash,
            "root_dir_id": graph.root_dir_id,
            "index": "graph/index/by-id.json",
        });
        write_json_file(&self.refs_dir().join("current.json"), &current_ref)?;

        Ok(root_graph_hash)
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    fn ensure_dirs(&self) -> Result<(), KnowledgeError> {
        for dir in [
            self.refs_dir(),
            self.index_dir(),
            self.objects_dir(),
            self.blobs_dir(),
        ] {
            fs::create_dir_all(&dir).map_err(|e| {
                KnowledgeError::knowledge_unavailable(format!("create dir {}: {e}", dir.display()))
            })?;
        }
        Ok(())
    }

    fn read_blob(&self, blob_hash: &str) -> Result<String, KnowledgeError> {
        let path = self.blob_path(blob_hash)?;
        let content = fs::read_to_string(&path).map_err(|e| {
            KnowledgeError::knowledge_unavailable(format!("read blob {}: {e}", path.display()))
        })?;
        let actual_hash = sha256_hex(content.as_bytes());
        if actual_hash != blob_hash {
            return Err(KnowledgeError::invalid_data(format!(
                "blob hash mismatch for {}: expected `{blob_hash}`, got `{actual_hash}`",
                path.display()
            )));
        }
        Ok(content)
    }

    fn write_json_object(&self, payload: &Value) -> Result<String, KnowledgeError> {
        let canonical = canonical_json(payload);
        let digest = sha256_hex(canonical.as_bytes());
        let path = self.object_path(&digest)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                KnowledgeError::knowledge_unavailable(format!(
                    "create dir {}: {e}",
                    parent.display()
                ))
            })?;
        }
        let sorted = sort_json_value(payload.clone());
        let pretty = serde_json::to_string_pretty(&sorted)
            .map_err(|e| KnowledgeError::invalid_data(format!("serialize object: {e}")))?;
        fs::write(&path, format!("{pretty}\n")).map_err(|e| {
            KnowledgeError::knowledge_unavailable(format!("write object {}: {e}", path.display()))
        })?;
        Ok(digest)
    }

    fn write_blob(&self, content: &str) -> Result<String, KnowledgeError> {
        let digest = sha256_hex(content.as_bytes());
        let path = self.blob_path(&digest)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                KnowledgeError::knowledge_unavailable(format!(
                    "create dir {}: {e}",
                    parent.display()
                ))
            })?;
        }
        fs::write(&path, content).map_err(|e| {
            KnowledgeError::knowledge_unavailable(format!("write blob {}: {e}", path.display()))
        })?;
        Ok(digest)
    }

    fn read_object_envelope(&self, object_hash: &str) -> Result<ObjectEnvelope, KnowledgeError> {
        let path = self.object_path(object_hash)?;
        let raw = fs::read_to_string(&path).map_err(|e| {
            KnowledgeError::knowledge_unavailable(format!("read object {}: {e}", path.display()))
        })?;
        let value: Value = serde_json::from_str(&raw)
            .map_err(|e| KnowledgeError::invalid_data(format!("parse {}: {e}", path.display())))?;
        let actual_hash = sha256_hex(canonical_json(&value).as_bytes());
        if actual_hash != object_hash {
            return Err(KnowledgeError::invalid_data(format!(
                "object hash mismatch for {}: expected `{object_hash}`, got `{actual_hash}`",
                path.display()
            )));
        }
        serde_json::from_value(value)
            .map_err(|e| KnowledgeError::invalid_data(format!("parse {}: {e}", path.display())))
    }
}

// ---------------------------------------------------------------------------
// Free-standing file I/O helpers
// ---------------------------------------------------------------------------

fn read_json_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, KnowledgeError> {
    let content = fs::read_to_string(path).map_err(|e| {
        KnowledgeError::knowledge_unavailable(format!("read {}: {e}", path.display()))
    })?;
    serde_json::from_str(&content)
        .map_err(|e| KnowledgeError::invalid_data(format!("parse {}: {e}", path.display())))
}

fn write_json_file(path: &Path, payload: &Value) -> Result<(), KnowledgeError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            KnowledgeError::knowledge_unavailable(format!("create dir {}: {e}", parent.display()))
        })?;
    }
    let sorted = sort_json_value(payload.clone());
    let pretty = serde_json::to_string_pretty(&sorted)
        .map_err(|e| KnowledgeError::invalid_data(format!("serialize json: {e}")))?;
    fs::write(path, format!("{pretty}\n")).map_err(|e| {
        KnowledgeError::knowledge_unavailable(format!("write json {}: {e}", path.display()))
    })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Canonical JSON (must match Python's json.dumps(sort_keys=True, separators=(",",":")))
// ---------------------------------------------------------------------------

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

fn dir_depth(location: &str) -> usize {
    let trimmed = location.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "." {
        0
    } else {
        Path::new(trimmed).components().count()
    }
}

fn sort_json_value(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<String, Value> = map
                .into_iter()
                .map(|(k, v)| (k, sort_json_value(v)))
                .collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(sort_json_value).collect()),
        other => other,
    }
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
