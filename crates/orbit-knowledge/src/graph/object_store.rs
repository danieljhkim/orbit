//! Content-addressed graph persistence for `.orbit/knowledge/graph/`.
//!
//! Reads and writes the same on-disk format as Python's `orbit_map/graph/store.py`.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use orbit_common::types::OrbitError;
use orbit_common::utility::git::{
    CurrentBranchStatus, current_branch as git_current_branch, default_branch as git_default_branch,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use super::nodes::{CodebaseGraphV1, DirNode, FileNode, LeafNode};
use crate::error::KnowledgeError;
use crate::io::{write_text_atomic, write_text_atomic_durable};

const GRAPH_STORE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RefName(String);

impl RefName {
    pub fn new(value: impl Into<String>) -> Result<Self, KnowledgeError> {
        let value = value.into();
        validate_ref_name(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RefName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentRef {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub root_graph_hash: String,
    #[serde(default)]
    pub root_object_hash: String,
    #[serde(default)]
    pub root_dir_id: String,
    pub index: String,
    /// Newest commit SHA walked by the attribute-history pipeline stage
    /// (T20260421-0528). `None` on pre-T0528 refs triggers a one-time
    /// full-history backfill on next rebuild.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attributed_commit: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedGraphRef {
    pub requested: RefName,
    pub resolved: RefName,
    pub ref_path: PathBuf,
    pub index_path: PathBuf,
    pub current_ref: CurrentRef,
    pub used_fallback: bool,
}

#[derive(Debug, Clone)]
pub struct GraphReadTarget {
    pub requested: RefName,
    pub fallback: Option<RefName>,
    pub default: Option<RefName>,
}

#[derive(Debug, Clone)]
pub struct GraphWriteTarget {
    pub requested: RefName,
    pub default: Option<RefName>,
}

pub fn resolve_graph_read_target(
    workspace_path: Option<&Path>,
    explicit_ref: Option<&str>,
) -> Result<GraphReadTarget, OrbitError> {
    let explicit_ref = explicit_ref.filter(|value| !value.trim().is_empty());
    let default = match workspace_path {
        Some(path) => resolve_default_ref(path)?,
        None => None,
    };

    if let Some(explicit_ref) = explicit_ref {
        return Ok(GraphReadTarget {
            requested: parse_ref_name(explicit_ref)?,
            fallback: None,
            default,
        });
    }

    let Some(workspace_path) = workspace_path else {
        return Err(OrbitError::InvalidInput(
            "`ref` is required when workspace_root is unavailable".to_string(),
        ));
    };

    let requested = resolve_current_ref(workspace_path)?;
    let fallback = default
        .as_ref()
        .filter(|default_ref| *default_ref != &requested)
        .cloned();

    Ok(GraphReadTarget {
        requested,
        fallback,
        default,
    })
}

pub fn resolve_graph_write_target(
    workspace_path: &Path,
    explicit_ref: Option<&str>,
) -> Result<GraphWriteTarget, OrbitError> {
    let requested = match explicit_ref.filter(|value| !value.trim().is_empty()) {
        Some(explicit_ref) => parse_ref_name(explicit_ref)?,
        None => resolve_current_ref(workspace_path)?,
    };

    Ok(GraphWriteTarget {
        requested,
        default: resolve_default_ref(workspace_path)?,
    })
}

fn parse_ref_name(value: &str) -> Result<RefName, OrbitError> {
    RefName::new(value).map_err(|error| OrbitError::InvalidInput(error.to_string()))
}

fn resolve_current_ref(workspace_path: &Path) -> Result<RefName, OrbitError> {
    match git_current_branch(workspace_path)? {
        CurrentBranchStatus::Named(branch) => {
            RefName::new(branch).map_err(|error| OrbitError::InvalidInput(error.to_string()))
        }
        CurrentBranchStatus::DetachedHead => Err(OrbitError::Execution(format!(
            "workspace '{}' is in detached HEAD; pass `--ref <name>` to select a knowledge-graph ref",
            workspace_path.display()
        ))),
        CurrentBranchStatus::NoCurrentBranch => Err(OrbitError::Execution(format!(
            "workspace '{}' has no current branch; pass `--ref <name>` to select a knowledge-graph ref",
            workspace_path.display()
        ))),
    }
}

fn resolve_default_ref(workspace_path: &Path) -> Result<Option<RefName>, OrbitError> {
    git_default_branch(workspace_path)?
        .map(RefName::new)
        .transpose()
        .map_err(|error| {
            OrbitError::Execution(format!(
                "resolved default branch for '{}' is invalid: {error}",
                workspace_path.display()
            ))
        })
}

// ---------------------------------------------------------------------------
// Internal deserialization types
// ---------------------------------------------------------------------------

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

    fn refs_heads_dir(&self) -> PathBuf {
        self.refs_dir().join("heads")
    }

    fn legacy_ref_path(&self) -> PathBuf {
        self.refs_dir().join("current.json")
    }

    fn index_dir(&self) -> PathBuf {
        self.graph_dir.join("index").join("by-id")
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

    fn index_path_for_hash(&self, root_graph_hash: &str) -> Result<PathBuf, KnowledgeError> {
        Ok(self.index_dir().join(format!("{root_graph_hash}.json")))
    }

    fn resolve_index_path(&self, index_ref: &str) -> Result<PathBuf, KnowledgeError> {
        let validated = validate_relative_path(index_ref)?;
        let graph_relative = validated.strip_prefix("graph").map_err(|_| {
            KnowledgeError::invalid_data(format!(
                "graph index path `{index_ref}` must be knowledge-root-relative and start with `graph/`"
            ))
        })?;
        if graph_relative.as_os_str().is_empty() {
            return Err(KnowledgeError::invalid_data(format!(
                "graph index path `{index_ref}` resolves to an empty graph-relative path"
            )));
        }
        Ok(self.graph_dir.join(graph_relative))
    }

    pub fn ref_path(&self, name: &RefName) -> PathBuf {
        self.refs_heads_dir().join(format!("{name}.json"))
    }

    pub fn prepare_refs_layout(&self, default_ref: Option<&RefName>) -> Result<(), KnowledgeError> {
        fs::create_dir_all(self.refs_heads_dir()).map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!(
                "create refs dir {}: {error}",
                self.refs_heads_dir().display()
            ))
        })?;

        let legacy_path = self.legacy_ref_path();
        if !legacy_path.is_file() {
            return Ok(());
        }

        let Some(default_ref) = default_ref else {
            return Err(KnowledgeError::knowledge_unavailable(format!(
                "legacy graph ref exists at {} but the repo default branch could not be resolved",
                legacy_path.display()
            )));
        };

        let migrated_path = self.ref_path(default_ref);
        if let Some(parent) = migrated_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                KnowledgeError::knowledge_unavailable(format!(
                    "create refs dir {}: {error}",
                    parent.display()
                ))
            })?;
        }

        if migrated_path.is_file() {
            warn!(
                "legacy knowledge graph ref exists alongside branch ref: {} and {}",
                legacy_path.display(),
                migrated_path.display()
            );
            fs::remove_file(&legacy_path).map_err(|error| {
                KnowledgeError::knowledge_unavailable(format!(
                    "remove legacy graph ref {}: {error}",
                    legacy_path.display()
                ))
            })?;
            return Ok(());
        }

        fs::rename(&legacy_path, &migrated_path).map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!(
                "migrate graph ref {} -> {}: {error}",
                legacy_path.display(),
                migrated_path.display()
            ))
        })?;
        info!(
            "migrated knowledge graph ref: {} -> {}",
            legacy_path.display(),
            migrated_path.display()
        );
        Ok(())
    }

    pub fn read_ref(&self, name: &RefName) -> Result<CurrentRef, KnowledgeError> {
        let ref_path = self.ref_path(name);
        read_json_file(&ref_path)
    }

    pub fn write_ref_atomic(
        &self,
        name: &RefName,
        current_ref: &CurrentRef,
    ) -> Result<(), KnowledgeError> {
        let payload = serde_json::to_value(current_ref).map_err(|error| {
            KnowledgeError::invalid_data(format!("serialize current ref: {error}"))
        })?;
        write_json_file(&self.ref_path(name), &payload)
    }

    pub fn resolve_ref(
        &self,
        requested: &RefName,
        fallback: Option<&RefName>,
    ) -> Result<ResolvedGraphRef, KnowledgeError> {
        let requested_path = self.ref_path(requested);
        if requested_path.is_file() {
            return self.load_resolved_ref(
                requested.clone(),
                requested.clone(),
                requested_path,
                false,
            );
        }

        if let Some(fallback) = fallback
            && fallback != requested
        {
            let fallback_path = self.ref_path(fallback);
            if fallback_path.is_file() {
                eprintln!(
                    "warning: knowledge graph ref for branch '{}' is missing; falling back to default branch '{}'",
                    requested, fallback
                );
                return self.load_resolved_ref(
                    requested.clone(),
                    fallback.clone(),
                    fallback_path,
                    true,
                );
            }
        }

        Err(KnowledgeError::knowledge_unavailable(format!(
            "graph reference for branch `{}` is unavailable at {}",
            requested,
            requested_path.display()
        )))
    }

    fn load_resolved_ref(
        &self,
        requested: RefName,
        resolved: RefName,
        ref_path: PathBuf,
        used_fallback: bool,
    ) -> Result<ResolvedGraphRef, KnowledgeError> {
        let current_ref: CurrentRef = read_json_file(&ref_path)?;
        let index_path = self.resolve_index_path(&current_ref.index)?;
        Ok(ResolvedGraphRef {
            requested,
            resolved,
            ref_path,
            index_path,
            current_ref,
            used_fallback,
        })
    }

    // -----------------------------------------------------------------------
    // Read path
    // -----------------------------------------------------------------------

    pub fn read_graph(
        &self,
        requested: &RefName,
        fallback: Option<&RefName>,
        default_ref: Option<&RefName>,
    ) -> Result<CodebaseGraphV1, KnowledgeError> {
        self.prepare_refs_layout(default_ref)?;
        let resolved = self.resolve_ref(requested, fallback)?;
        let index: ByIdIndex = read_json_file(&resolved.index_path)?;

        let mut dirs = Vec::with_capacity(index.dirs.len());
        for node_id in &index.dirs {
            let entry = index.nodes.get(node_id).ok_or_else(|| {
                KnowledgeError::invalid_data(format!("dir node {node_id} missing from index"))
            })?;
            let envelope = self.read_object_envelope(&entry.object_hash)?;
            let mut dir: DirNode = serde_json::from_value(envelope.node).map_err(|error| {
                KnowledgeError::invalid_data(format!("dir node parse: {error}"))
            })?;
            dir.base.object_hash = Some(entry.object_hash.clone());
            dirs.push(dir);
        }

        let mut files = Vec::with_capacity(index.files.len());
        for node_id in &index.files {
            let entry = index.nodes.get(node_id).ok_or_else(|| {
                KnowledgeError::invalid_data(format!("file node {node_id} missing from index"))
            })?;
            let envelope = self.read_object_envelope(&entry.object_hash)?;
            let mut file: FileNode = serde_json::from_value(envelope.node).map_err(|error| {
                KnowledgeError::invalid_data(format!("file node parse: {error}"))
            })?;
            file.base.object_hash = Some(entry.object_hash.clone());
            files.push(file);
        }

        let mut leaves = Vec::with_capacity(index.leaves.len());
        for node_id in &index.leaves {
            let entry = index.nodes.get(node_id).ok_or_else(|| {
                KnowledgeError::invalid_data(format!("leaf node {node_id} missing from index"))
            })?;
            let envelope = self.read_object_envelope(&entry.object_hash)?;
            let mut leaf: LeafNode = serde_json::from_value(envelope.node).map_err(|error| {
                KnowledgeError::invalid_data(format!("leaf node parse: {error}"))
            })?;
            leaf.base.object_hash = Some(entry.object_hash.clone());

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

    pub fn write_graph(&self, graph: &CodebaseGraphV1) -> Result<CurrentRef, KnowledgeError> {
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

        for leaf in &graph.leaves {
            let mut node_json = serde_json::to_value(leaf).map_err(|error| {
                KnowledgeError::invalid_data(format!("leaf serialize: {error}"))
            })?;

            if let Value::Object(ref mut map) = node_json {
                map.remove("object_hash");
            }

            if !leaf.source.is_empty() {
                let blob_hash = self.write_blob(&leaf.source)?;
                if let Value::Object(ref mut map) = node_json {
                    map.insert("source_blob_hash".to_string(), Value::String(blob_hash));
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
                    // LeafKind has data-carrying variants (e.g. `Section { depth }`)
                    // since T20260422-1540. The index stores the Display form
                    // (bare string) so downstream selector lookup stays simple;
                    // the struct-variant payload lives in the full object body.
                    "kind": leaf.kind.to_string(),
                }),
            );
        }

        for file in &graph.files {
            let mut node_json = serde_json::to_value(file).map_err(|error| {
                KnowledgeError::invalid_data(format!("file serialize: {error}"))
            })?;
            if let Value::Object(ref mut map) = node_json {
                map.remove("object_hash");
            }

            let child_hashes: HashMap<String, String> = file
                .leaf_children
                .iter()
                .map(|child_id| {
                    let hash = object_hashes.get(child_id).ok_or_else(|| {
                        KnowledgeError::invalid_data(format!(
                            "file `{}` references missing child `{child_id}`",
                            file.base.id
                        ))
                    })?;
                    Ok((child_id.clone(), hash.clone()))
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

        let mut dirs_in_write_order: Vec<&DirNode> = graph.dirs.iter().collect();
        dirs_in_write_order.sort_by(|a, b| {
            dir_depth(&b.base.location)
                .cmp(&dir_depth(&a.base.location))
                .then_with(|| a.base.location.cmp(&b.base.location))
        });

        for dir in dirs_in_write_order {
            let mut node_json = serde_json::to_value(dir)
                .map_err(|error| KnowledgeError::invalid_data(format!("dir serialize: {error}")))?;
            if let Value::Object(ref mut map) = node_json {
                map.remove("object_hash");
            }

            let mut child_hashes: HashMap<String, String> = HashMap::new();
            for child_id in &dir.dir_children {
                let hash = object_hashes.get(child_id).ok_or_else(|| {
                    KnowledgeError::invalid_data(format!(
                        "dir `{}` references missing dir child `{child_id}`",
                        dir.base.id
                    ))
                })?;
                child_hashes.insert(child_id.clone(), hash.clone());
            }
            for child_id in &dir.file_children {
                let hash = object_hashes.get(child_id).ok_or_else(|| {
                    KnowledgeError::invalid_data(format!(
                        "dir `{}` references missing file child `{child_id}`",
                        dir.base.id
                    ))
                })?;
                child_hashes.insert(child_id.clone(), hash.clone());
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

        let root_object_hash = object_hashes
            .get(&graph.root_dir_id)
            .cloned()
            .ok_or_else(|| {
                KnowledgeError::invalid_data(format!(
                    "graph root `{}` did not produce an object hash",
                    graph.root_dir_id
                ))
            })?;

        let root_payload = serde_json::json!({
            "schema_version": GRAPH_STORE_SCHEMA_VERSION,
            "object_type": "codebase_graph",
            "root_dir_id": graph.root_dir_id,
            "root_object_hash": &root_object_hash,
            "dirs": graph.dirs.iter().map(|d| &d.base.id).collect::<Vec<_>>(),
            "files": graph.files.iter().map(|f| &f.base.id).collect::<Vec<_>>(),
            "leaves": graph.leaves.iter().map(|l| &l.base.id).collect::<Vec<_>>(),
            "node_count": graph.dirs.len() + graph.files.len() + graph.leaves.len(),
        });
        let root_graph_hash = self.write_json_object(&root_payload)?;

        let by_id_index = serde_json::json!({
            "schema_version": GRAPH_STORE_SCHEMA_VERSION,
            "root_dir_id": graph.root_dir_id,
            "root_graph_hash": &root_graph_hash,
            "root_object_hash": &root_object_hash,
            "dirs": graph.dirs.iter().map(|d| &d.base.id).collect::<Vec<_>>(),
            "files": graph.files.iter().map(|f| &f.base.id).collect::<Vec<_>>(),
            "leaves": graph.leaves.iter().map(|l| &l.base.id).collect::<Vec<_>>(),
            "nodes": index_nodes,
        });
        write_json_file(&self.index_path_for_hash(&root_graph_hash)?, &by_id_index)?;

        // The stored `index` path is knowledge-root-relative. Readers rooted at
        // `knowledge_dir` can join it directly; readers rooted at `graph_dir`
        // strip the leading `graph/` explicitly and locally.
        Ok(CurrentRef {
            schema_version: GRAPH_STORE_SCHEMA_VERSION,
            root_graph_hash: root_graph_hash.clone(),
            root_object_hash,
            root_dir_id: graph.root_dir_id.clone(),
            index: format!("graph/index/by-id/{root_graph_hash}.json"),
            last_attributed_commit: None,
        })
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    fn ensure_dirs(&self) -> Result<(), KnowledgeError> {
        for dir in [
            self.refs_heads_dir(),
            self.index_dir(),
            self.objects_dir(),
            self.blobs_dir(),
        ] {
            fs::create_dir_all(&dir).map_err(|error| {
                KnowledgeError::knowledge_unavailable(format!(
                    "create dir {}: {error}",
                    dir.display()
                ))
            })?;
        }
        Ok(())
    }

    fn read_blob(&self, blob_hash: &str) -> Result<String, KnowledgeError> {
        let path = self.blob_path(blob_hash)?;
        let content = fs::read_to_string(&path).map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!("read blob {}: {error}", path.display()))
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
        let sorted = sort_json_value(payload.clone());
        let pretty = serde_json::to_string_pretty(&sorted)
            .map_err(|error| KnowledgeError::invalid_data(format!("serialize object: {error}")))?;
        write_text_atomic(&path, &format!("{pretty}\n")).map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!(
                "write object {}: {error}",
                path.display()
            ))
        })?;
        Ok(digest)
    }

    fn write_blob(&self, content: &str) -> Result<String, KnowledgeError> {
        let digest = sha256_hex(content.as_bytes());
        let path = self.blob_path(&digest)?;
        write_text_atomic(&path, content).map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!("write blob {}: {error}", path.display()))
        })?;
        Ok(digest)
    }

    fn read_object_envelope(&self, object_hash: &str) -> Result<ObjectEnvelope, KnowledgeError> {
        let path = self.object_path(object_hash)?;
        let raw = fs::read_to_string(&path).map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!(
                "read object {}: {error}",
                path.display()
            ))
        })?;
        let value: Value = serde_json::from_str(&raw).map_err(|error| {
            KnowledgeError::invalid_data(format!("parse {}: {error}", path.display()))
        })?;
        let actual_hash = sha256_hex(canonical_json(&value).as_bytes());
        if actual_hash != object_hash {
            return Err(KnowledgeError::invalid_data(format!(
                "object hash mismatch for {}: expected `{object_hash}`, got `{actual_hash}`",
                path.display()
            )));
        }
        serde_json::from_value(value).map_err(|error| {
            KnowledgeError::invalid_data(format!("parse {}: {error}", path.display()))
        })
    }
}

// ---------------------------------------------------------------------------
// Free-standing file I/O helpers
// ---------------------------------------------------------------------------

fn read_json_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, KnowledgeError> {
    let content = fs::read_to_string(path).map_err(|error| {
        KnowledgeError::knowledge_unavailable(format!("read {}: {error}", path.display()))
    })?;
    serde_json::from_str(&content)
        .map_err(|error| KnowledgeError::invalid_data(format!("parse {}: {error}", path.display())))
}

pub(crate) fn validate_relative_path(path: &str) -> Result<PathBuf, KnowledgeError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(KnowledgeError::invalid_data(
            "graph path must not be empty".to_string(),
        ));
    }

    let rel_path = Path::new(trimmed);
    if rel_path.is_absolute()
        || rel_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(KnowledgeError::invalid_data(format!(
            "invalid graph path `{path}`"
        )));
    }
    Ok(rel_path.to_path_buf())
}

fn validate_ref_name(value: &str) -> Result<(), KnowledgeError> {
    if value.trim().is_empty() {
        return Err(KnowledgeError::invalid_data(
            "graph ref name must not be empty".to_string(),
        ));
    }
    if value.starts_with('-') {
        return Err(KnowledgeError::invalid_data(format!(
            "graph ref name `{value}` must not start with `-`"
        )));
    }
    if value.starts_with('/') || value.ends_with('/') || value.contains("//") {
        return Err(KnowledgeError::invalid_data(format!(
            "graph ref name `{value}` must not start or end with `/` or contain `//`"
        )));
    }
    if value.contains("..")
        || value.contains("@{")
        || value.ends_with('.')
        || value.ends_with(".lock")
    {
        return Err(KnowledgeError::invalid_data(format!(
            "graph ref name `{value}` is not a valid git branch name"
        )));
    }
    if value.chars().any(|ch| {
        ch.is_whitespace()
            || ch.is_control()
            || matches!(ch, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
    }) {
        return Err(KnowledgeError::invalid_data(format!(
            "graph ref name `{value}` is not a valid git branch name"
        )));
    }
    if Path::new(value)
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(KnowledgeError::invalid_data(format!(
            "graph ref name `{value}` is not a valid git branch name"
        )));
    }
    Ok(())
}

fn write_json_file(path: &Path, payload: &Value) -> Result<(), KnowledgeError> {
    let sorted = sort_json_value(payload.clone());
    let pretty = serde_json::to_string_pretty(&sorted)
        .map_err(|error| KnowledgeError::invalid_data(format!("serialize json: {error}")))?;
    write_text_atomic_durable(path, &format!("{pretty}\n")).map_err(|error| {
        KnowledgeError::knowledge_unavailable(format!("write json {}: {error}", path.display()))
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

/// Depth of a directory location, with the repo root at depth 0.
///
/// Callers pass the `DirNode.location` string, which `build_graph_dirs`
/// formats as `"<path>/"` — the trailing slash is part of every location.
/// The root dir ends up as `"./"`, which must normalize to depth 0 so that
/// `write_graph`'s depth-descending sort writes children before their parents.
/// The pre-fix implementation treated the leading `.` as a segment and
/// returned 1, which tied root with its top-level children and caused
/// `dir references missing dir child` errors during serialization (T20260421-0652).
fn dir_depth(location: &str) -> usize {
    let trimmed = location.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "." {
        return 0;
    }
    trimmed
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_depth_root_location_is_zero() {
        // Regression for T20260421-0652: root location produced by
        // `build_graph_dirs` is `"./"`, which must normalize to depth 0 so the
        // depth-descending sort in `write_graph` writes root after its
        // children.
        assert_eq!(dir_depth("./"), 0);
        assert_eq!(dir_depth("."), 0);
        assert_eq!(dir_depth(""), 0);
        assert_eq!(dir_depth("/"), 0);
    }

    #[test]
    fn dir_depth_counts_segments_not_slashes() {
        assert_eq!(dir_depth("src/"), 1);
        assert_eq!(dir_depth("src"), 1);
        assert_eq!(dir_depth("src/foo/"), 2);
        assert_eq!(dir_depth("src/foo/bar/"), 3);
    }

    #[test]
    fn dir_depth_ignores_current_dir_segments() {
        // Paths like "./src/" should count "src" only — the leading "." is a
        // relative-path marker, not a depth segment.
        assert_eq!(dir_depth("./src/"), 1);
        assert_eq!(dir_depth("./src/foo/"), 2);
    }

    #[test]
    fn dir_depth_is_strict_weak_order_root_first_by_descending_depth() {
        // Depth-descending sort must place nested dirs before root.
        let mut locations = vec!["./", "src/", "src/foo/"];
        locations.sort_by(|a, b| dir_depth(b).cmp(&dir_depth(a)));
        assert_eq!(locations, vec!["src/foo/", "src/", "./"]);
    }
}
