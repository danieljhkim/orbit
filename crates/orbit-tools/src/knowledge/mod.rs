use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Error)]
#[error("{kind}: {reason}")]
pub struct KnowledgeError {
    pub kind: String,
    pub reason: String,
}

impl KnowledgeError {
    fn knowledge_unavailable(reason: impl Into<String>) -> Self {
        Self {
            kind: "knowledge_unavailable".to_string(),
            reason: reason.into(),
        }
    }

    fn invalid_data(reason: impl Into<String>) -> Self {
        Self {
            kind: "knowledge_invalid".to_string(),
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Error)]
#[error("invalid selector `{selector}`: {reason}")]
pub struct SelectorParseError {
    pub selector: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeEntryKind {
    Dir,
    File,
    Leaf,
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KnowledgePackEntry {
    pub selector: String,
    pub resolved_id: Option<String>,
    pub kind: KnowledgeEntryKind,
    pub content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KnowledgePack {
    pub knowledge_dir: String,
    pub manifest_generated_at: String,
    pub unresolved_selectors: Vec<String>,
    pub total_nodes: usize,
    pub entries: Vec<KnowledgePackEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    Dir {
        path: String,
    },
    File {
        path: String,
    },
    Leaf {
        path: String,
        symbol: String,
        kind: String,
    },
}

impl Selector {
    pub fn parse_many(raw_selectors: &[String]) -> Result<Vec<Self>, SelectorParseError> {
        raw_selectors
            .iter()
            .map(|selector| selector.parse())
            .collect()
    }

    fn lookup_key(&self) -> SelectorLookupKey {
        match self {
            Self::Dir { path } => SelectorLookupKey::Dir(path.clone()),
            Self::File { path } => SelectorLookupKey::File(path.clone()),
            Self::Leaf { path, symbol, kind } => {
                SelectorLookupKey::Leaf(format!("{path}#{symbol}"), kind.clone())
            }
        }
    }
}

impl std::fmt::Display for Selector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dir { path } => write!(f, "dir:{path}"),
            Self::File { path } => write!(f, "file:{path}"),
            Self::Leaf { path, symbol, kind } => write!(f, "leaf:{path}#{symbol}:{kind}"),
        }
    }
}

impl FromStr for Selector {
    type Err = SelectorParseError;

    fn from_str(selector: &str) -> Result<Self, Self::Err> {
        let trimmed = selector.trim();
        if let Some(path) = trimmed.strip_prefix("dir:") {
            if path.is_empty() {
                return Err(SelectorParseError {
                    selector: selector.to_string(),
                    reason: "dir selector path must not be empty".to_string(),
                });
            }
            return Ok(Self::Dir {
                path: path.to_string(),
            });
        }

        if let Some(path) = trimmed.strip_prefix("file:") {
            if path.is_empty() {
                return Err(SelectorParseError {
                    selector: selector.to_string(),
                    reason: "file selector path must not be empty".to_string(),
                });
            }
            return Ok(Self::File {
                path: path.to_string(),
            });
        }

        if let Some(remainder) = trimmed.strip_prefix("leaf:") {
            let (location, kind) =
                remainder
                    .rsplit_once(':')
                    .ok_or_else(|| SelectorParseError {
                        selector: selector.to_string(),
                        reason: "leaf selectors must use `leaf:<path>#<symbol>:<kind>`".to_string(),
                    })?;
            if location.is_empty() || kind.is_empty() {
                return Err(SelectorParseError {
                    selector: selector.to_string(),
                    reason: "leaf selectors must include both a location and kind".to_string(),
                });
            }
            let (path, symbol) = location.split_once('#').ok_or_else(|| SelectorParseError {
                selector: selector.to_string(),
                reason: "leaf selectors must include `#<symbol>`".to_string(),
            })?;
            if path.is_empty() || symbol.is_empty() {
                return Err(SelectorParseError {
                    selector: selector.to_string(),
                    reason: "leaf selectors must include non-empty path and symbol".to_string(),
                });
            }
            return Ok(Self::Leaf {
                path: path.to_string(),
                symbol: symbol.to_string(),
                kind: kind.to_string(),
            });
        }

        Err(SelectorParseError {
            selector: selector.to_string(),
            reason: "selectors must start with `dir:`, `file:`, or `leaf:`".to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum SelectorLookupKey {
    Dir(String),
    File(String),
    Leaf(String, String),
}

#[derive(Debug, Clone)]
pub struct KnowledgeStore {
    knowledge_dir: PathBuf,
    manifest: ManifestFile,
    graph_index: GraphIndexFile,
    selector_index: HashMap<SelectorLookupKey, String>,
}

impl KnowledgeStore {
    pub fn open(knowledge_dir: &Path) -> Result<Self, KnowledgeError> {
        if !knowledge_dir.is_dir() {
            return Err(KnowledgeError::knowledge_unavailable(format!(
                "knowledge directory does not exist: {}",
                knowledge_dir.display()
            )));
        }

        let manifest_path = knowledge_dir.join("manifest.json");
        let manifest: ManifestFile = read_json_file(&manifest_path).map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!(
                "manifest.json is unavailable or invalid at {}: {error}",
                manifest_path.display()
            ))
        })?;

        let current_ref_path = knowledge_dir.join("graph/refs/current.json");
        let current_ref: CurrentRefFile = read_json_file(&current_ref_path).map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!(
                "graph reference is unavailable or invalid at {}: {error}",
                current_ref_path.display()
            ))
        })?;

        let graph_index_path = knowledge_dir.join(&current_ref.index);
        let graph_index: GraphIndexFile = read_json_file(&graph_index_path).map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!(
                "graph index is unavailable or invalid at {}: {error}",
                graph_index_path.display()
            ))
        })?;

        let mut selector_index = HashMap::new();
        for (node_id, entry) in &graph_index.nodes {
            let key = match entry.node_type.as_str() {
                "dir" => SelectorLookupKey::Dir(entry.location.clone()),
                "file" => SelectorLookupKey::File(entry.location.clone()),
                "leaf" => SelectorLookupKey::Leaf(
                    entry.location.clone(),
                    entry.kind.clone().ok_or_else(|| {
                        KnowledgeError::invalid_data(format!(
                            "leaf index entry `{node_id}` is missing a `kind`"
                        ))
                    })?,
                ),
                other => {
                    return Err(KnowledgeError::invalid_data(format!(
                        "unsupported graph node type `{other}` for `{node_id}`"
                    )));
                }
            };
            selector_index.insert(key, node_id.clone());
        }

        Ok(Self {
            knowledge_dir: knowledge_dir.to_path_buf(),
            manifest,
            graph_index,
            selector_index,
        })
    }

    pub fn is_available(knowledge_dir: &Path) -> bool {
        Self::open(knowledge_dir).is_ok()
    }

    pub fn pack(&self, selectors: &[Selector]) -> Result<KnowledgePack, KnowledgeError> {
        let mut object_cache = HashMap::<String, Value>::new();
        let mut blob_cache = HashMap::<String, String>::new();
        let mut entries = Vec::with_capacity(selectors.len());
        let mut unresolved_selectors = Vec::new();

        for selector in selectors {
            let selector_string = selector.to_string();
            let Some(node_id) = self.selector_index.get(&selector.lookup_key()).cloned() else {
                unresolved_selectors.push(selector_string.clone());
                entries.push(KnowledgePackEntry {
                    selector: selector_string,
                    resolved_id: None,
                    kind: KnowledgeEntryKind::Unresolved,
                    content: None,
                    source: None,
                });
                continue;
            };

            let index_entry = self.graph_index.nodes.get(&node_id).ok_or_else(|| {
                KnowledgeError::invalid_data(format!(
                    "graph index entry disappeared for `{node_id}`"
                ))
            })?;

            let content = read_graph_object(
                &self.knowledge_dir,
                &index_entry.object_hash,
                &mut object_cache,
            )?;
            let source = if index_entry.node_type == "leaf" {
                extract_leaf_source(&self.knowledge_dir, &content, &mut blob_cache)?
            } else {
                None
            };
            let kind = match index_entry.node_type.as_str() {
                "dir" => KnowledgeEntryKind::Dir,
                "file" => KnowledgeEntryKind::File,
                "leaf" => KnowledgeEntryKind::Leaf,
                other => {
                    return Err(KnowledgeError::invalid_data(format!(
                        "unsupported graph node type `{other}` for `{node_id}`"
                    )));
                }
            };

            entries.push(KnowledgePackEntry {
                selector: selector_string,
                resolved_id: Some(node_id),
                kind,
                content: Some(content),
                source,
            });
        }

        let total_nodes = entries
            .iter()
            .filter(|entry| entry.resolved_id.is_some())
            .count();
        Ok(KnowledgePack {
            knowledge_dir: self.knowledge_dir.display().to_string(),
            manifest_generated_at: self.manifest.generated_at.clone(),
            unresolved_selectors,
            total_nodes,
            entries,
        })
    }
}

fn read_graph_object(
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

fn extract_leaf_source(
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

fn read_json_file<T>(path: &Path) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&raw).map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Deserialize)]
struct ManifestFile {
    generated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CurrentRefFile {
    index: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GraphIndexFile {
    nodes: HashMap<String, GraphIndexEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct GraphIndexEntry {
    object_hash: String,
    node_type: String,
    location: String,
    kind: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use serde_json::json;

    use super::{
        KnowledgeEntryKind, KnowledgeError, KnowledgePack, KnowledgePackEntry, KnowledgeStore,
        Selector, SelectorParseError,
    };

    fn fixture_knowledge_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/knowledge")
            .join(".orbit/knowledge")
    }

    #[test]
    fn parses_supported_selector_forms() {
        assert_eq!(
            "dir:crates/orbit-tools/src".parse::<Selector>().unwrap(),
            Selector::Dir {
                path: "crates/orbit-tools/src".to_string()
            }
        );
        assert_eq!(
            "file:crates/orbit-tools/src/lib.rs"
                .parse::<Selector>()
                .unwrap(),
            Selector::File {
                path: "crates/orbit-tools/src/lib.rs".to_string()
            }
        );
        assert_eq!(
            "leaf:crates/orbit-tools/src/lib.rs#register_builtins:function"
                .parse::<Selector>()
                .unwrap(),
            Selector::Leaf {
                path: "crates/orbit-tools/src/lib.rs".to_string(),
                symbol: "register_builtins".to_string(),
                kind: "function".to_string(),
            }
        );
    }

    #[test]
    fn rejects_malformed_selectors() {
        assert_eq!(
            "".parse::<Selector>().unwrap_err(),
            SelectorParseError {
                selector: "".to_string(),
                reason: "selectors must start with `dir:`, `file:`, or `leaf:`".to_string(),
            }
        );
        assert_eq!(
            "file:".parse::<Selector>().unwrap_err(),
            SelectorParseError {
                selector: "file:".to_string(),
                reason: "file selector path must not be empty".to_string(),
            }
        );
        assert_eq!(
            "leaf:crates/orbit-tools/src/lib.rs#register_builtins"
                .parse::<Selector>()
                .unwrap_err(),
            SelectorParseError {
                selector: "leaf:crates/orbit-tools/src/lib.rs#register_builtins".to_string(),
                reason: "leaf selectors must use `leaf:<path>#<symbol>:<kind>`".to_string(),
            }
        );
    }

    #[test]
    fn open_reports_missing_knowledge_directory() {
        let missing = fixture_knowledge_dir().join("missing");

        let error = KnowledgeStore::open(&missing).unwrap_err();

        assert_eq!(
            error,
            KnowledgeError {
                kind: "knowledge_unavailable".to_string(),
                reason: format!("knowledge directory does not exist: {}", missing.display()),
            }
        );
    }

    #[test]
    fn open_reports_corrupt_manifest() {
        let corrupt_dir = tempfile::tempdir().expect("tempdir");
        let knowledge_dir = corrupt_dir.path().join(".orbit/knowledge");
        std::fs::create_dir_all(&knowledge_dir).expect("knowledge dir");
        std::fs::write(knowledge_dir.join("manifest.json"), "{not-json").expect("manifest");

        let error = KnowledgeStore::open(&knowledge_dir).unwrap_err();

        assert_eq!(error.kind, "knowledge_unavailable");
        assert!(!error.reason.is_empty());
        assert!(error.reason.contains("manifest.json"));
    }

    #[test]
    fn availability_checks_manifest_and_index() {
        assert!(KnowledgeStore::is_available(&fixture_knowledge_dir()));
        assert!(!KnowledgeStore::is_available(Path::new(
            "/tmp/definitely-missing-orbit"
        )));
    }

    #[test]
    fn pack_returns_exact_fixture_output() {
        let store = KnowledgeStore::open(&fixture_knowledge_dir()).expect("store");
        let selectors = Selector::parse_many(&[
            "file:crates/orbit-tools/src/lib.rs".to_string(),
            "leaf:crates/orbit-tools/src/lib.rs#register_builtins:function".to_string(),
        ])
        .expect("selectors");

        let pack = store.pack(&selectors).expect("pack");

        assert_eq!(
            pack,
            KnowledgePack {
                knowledge_dir: fixture_knowledge_dir().display().to_string(),
                manifest_generated_at: "2026-04-09T06:06:39Z".to_string(),
                unresolved_selectors: vec![],
                total_nodes: 2,
                entries: vec![
                    KnowledgePackEntry {
                        selector: "file:crates/orbit-tools/src/lib.rs".to_string(),
                        resolved_id: Some("node-file-lib".to_string()),
                        kind: KnowledgeEntryKind::File,
                        content: Some(json!({
                            "schema_version": 1,
                            "object_type": "graph_node",
                            "node_type": "file",
                            "node": {
                                "description": "Orbit tool registry facade.",
                                "exports": ["ToolRegistry"],
                                "extension": "rs",
                                "id": "node-file-lib",
                                "identity_key": "file:crates/orbit-tools/src/lib.rs",
                                "imports": ["orbit_lock::FileLockChecker", "serde_json::Value"],
                                "is_locked": false,
                                "language": "rust",
                                "leaf_children": ["node-leaf-register-builtins"],
                                "lineage_locked": false,
                                "location": "crates/orbit-tools/src/lib.rs",
                                "lock_owner": null,
                                "lock_reason": "",
                                "name": "lib.rs",
                                "parent_id": "node-dir-tools-src",
                                "source_blob_hash": "6666666666666666666666666666666666666666666666666666666666666666"
                            },
                            "child_object_hashes": {
                                "node-leaf-register-builtins": "3333333333333333333333333333333333333333333333333333333333333333"
                            }
                        })),
                        source: None,
                    },
                    KnowledgePackEntry {
                        selector: "leaf:crates/orbit-tools/src/lib.rs#register_builtins:function"
                            .to_string(),
                        resolved_id: Some("node-leaf-register-builtins".to_string()),
                        kind: KnowledgeEntryKind::Leaf,
                        content: Some(json!({
                            "schema_version": 1,
                            "object_type": "graph_node",
                            "node_type": "leaf",
                            "node": {
                                "children": [],
                                "description": "Registers the built-in orbit tools.",
                                "end_line": 22,
                                "file_hash_at_capture": "7777777777777777777777777777777777777777777777777777777777777777",
                                "history": [],
                                "id": "node-leaf-register-builtins",
                                "identity_key": "leaf:crates/orbit-tools/src/lib.rs#register_builtins:function",
                                "input_signature": [],
                                "is_locked": false,
                                "kind": "function",
                                "language": "rust",
                                "lineage_locked": false,
                                "location": "crates/orbit-tools/src/lib.rs#register_builtins",
                                "lock_owner": null,
                                "lock_reason": "",
                                "name": "register_builtins",
                                "output_signature": [],
                                "parent_id": "node-file-lib",
                                "source": "",
                                "source_blob_hash": "5555555555555555555555555555555555555555555555555555555555555555",
                                "source_hash": "8888888888888888888888888888888888888888888888888888888888888888",
                                "start_line": 11
                            },
                            "child_object_hashes": {}
                        })),
                        source: Some(
                            "pub fn register_builtins(registry: &mut ToolRegistry) {\n    fs::register(registry);\n    git::register(registry);\n}\n"
                                .to_string()
                        ),
                    },
                ],
            }
        );
    }

    #[test]
    fn pack_keeps_unresolved_selectors_out_of_error_path() {
        let store = KnowledgeStore::open(&fixture_knowledge_dir()).expect("store");
        let selectors = Selector::parse_many(&[
            "file:crates/orbit-tools/src/lib.rs".to_string(),
            "dir:crates/orbit-tools/src/missing".to_string(),
        ])
        .expect("selectors");

        let pack = store.pack(&selectors).expect("pack");

        assert_eq!(
            pack.unresolved_selectors,
            vec!["dir:crates/orbit-tools/src/missing".to_string()]
        );
        assert_eq!(pack.total_nodes, 1);
        assert_eq!(pack.entries[1].kind, KnowledgeEntryKind::Unresolved);
        assert_eq!(pack.entries[1].resolved_id, None);
        assert_eq!(pack.entries[1].content, None);
    }
}
