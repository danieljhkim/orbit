use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::KnowledgeError;
use crate::selector::Selector;
use crate::store::KnowledgeStore;

/// A single edit in a leaf's version chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeafEdit {
    pub edit_sequence: u32,
    pub source_hash: String,
    pub source: String,
    pub timestamp: String,
    pub reason: Option<String>,
}

/// Full version history for a leaf across an activity run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeafVersionChain {
    pub leaf_id: String,
    pub selector: String,
    pub original_source_hash: String,
    pub edits: Vec<LeafEdit>,
}

/// In-memory leaf state tracked by the working graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkingLeaf {
    pub selector: String,
    pub file_path: String,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub start_line: usize,
    pub end_line: usize,
    pub source: String,
    pub source_hash: String,
    pub parent_qualified_name: Option<String>,
    pub children_qualified_names: Vec<String>,
}

/// Result of a successful knowledge.write edit operation.
#[derive(Debug, Clone, Serialize)]
pub struct WriteResult {
    pub status: String,
    pub selector: String,
    pub edit_sequence: u32,
    pub new_source_hash: String,
    pub affected_leaves: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_leaf_id: Option<String>,
}

/// Error from a knowledge.write operation.
#[derive(Debug, Clone, Serialize)]
pub struct WriteError {
    pub kind: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_source_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leaf_id: Option<String>,
}

impl WriteError {
    pub fn source_conflict(expected_hash: &str, actual_source: &str, leaf_selector: &str) -> Self {
        Self {
            kind: "source_conflict".to_string(),
            reason: format!("file on disk does not match working graph for `{leaf_selector}`"),
            expected_source_hash: Some(expected_hash.to_string()),
            actual_source: Some(actual_source.to_string()),
            leaf_id: Some(leaf_selector.to_string()),
        }
    }

    pub fn position_not_found(selector: &str) -> Self {
        Self {
            kind: "position_not_found".to_string(),
            reason: format!("position reference selector `{selector}` does not resolve"),
            expected_source_hash: None,
            actual_source: None,
            leaf_id: None,
        }
    }

    pub fn unsupported_language(ext: &str) -> Self {
        Self {
            kind: "unsupported_language".to_string(),
            reason: format!("no extractor available for `.{ext}` files"),
            expected_source_hash: None,
            actual_source: None,
            leaf_id: None,
        }
    }

    pub fn io_error(reason: impl Into<String>) -> Self {
        Self {
            kind: "io_error".to_string(),
            reason: reason.into(),
            expected_source_hash: None,
            actual_source: None,
            leaf_id: None,
        }
    }

    pub fn leaf_already_exists(selector: &str) -> Self {
        Self {
            kind: "leaf_already_exists".to_string(),
            reason: format!("selector `{selector}` already exists; use graph.write to edit"),
            expected_source_hash: None,
            actual_source: None,
            leaf_id: Some(selector.to_string()),
        }
    }

    pub fn invalid_position(selector: &str, reason: impl Into<String>) -> Self {
        Self {
            kind: "invalid_position".to_string(),
            reason: format!("position selector `{selector}` {}", reason.into()),
            expected_source_hash: None,
            actual_source: None,
            leaf_id: Some(selector.to_string()),
        }
    }

    pub fn expected_leaf_not_found(selector: &str) -> Self {
        Self {
            kind: "expected_leaf_not_found".to_string(),
            reason: format!(
                "post-write extraction did not yield the expected leaf for `{selector}`"
            ),
            expected_source_hash: None,
            actual_source: None,
            leaf_id: Some(selector.to_string()),
        }
    }

    pub fn ambiguous_new_leaf(selector: &str, count: usize) -> Self {
        Self {
            kind: "ambiguous_new_leaf".to_string(),
            reason: format!("post-write extraction matched `{selector}` to {count} leaves"),
            expected_source_hash: None,
            actual_source: None,
            leaf_id: Some(selector.to_string()),
        }
    }

    pub fn duplicate_selector(selector: &str) -> Self {
        Self {
            kind: "duplicate_selector".to_string(),
            reason: format!("post-write extraction produced duplicate selector `{selector}`"),
            expected_source_hash: None,
            actual_source: None,
            leaf_id: Some(selector.to_string()),
        }
    }
}

/// Result of a move operation.
#[derive(Debug, Clone, Serialize)]
pub struct MoveResult {
    pub status: String,
    pub old_selector: String,
    pub new_selector: String,
    pub affected_leaves: Vec<String>,
}

/// In-memory working graph that tracks leaf state during an activity run.
///
/// Initialized from the persisted knowledge store, then mutated in memory
/// as `knowledge.write` calls modify files and re-extract.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkingGraph {
    /// Leaves indexed by selector string (e.g. "symbol:path#symbol:kind").
    pub(super) leaves: HashMap<String, WorkingLeaf>,
    /// Reverse index: file_path → list of leaf selector strings.
    pub(super) file_leaves: HashMap<String, Vec<String>>,
    /// Version chains indexed by leaf selector string.
    pub(super) version_chains: HashMap<String, LeafVersionChain>,
    /// Latest full-file content hashes used for stale-write detection.
    #[serde(default)]
    pub(super) file_snapshots: HashMap<String, String>,
}

impl WorkingGraph {
    /// Create a new empty working graph.
    pub fn new() -> Self {
        Self {
            leaves: HashMap::new(),
            file_leaves: HashMap::new(),
            version_chains: HashMap::new(),
            file_snapshots: HashMap::new(),
        }
    }

    /// Initialize a working graph from a persisted KnowledgeStore.
    ///
    /// Loads all leaf objects from the graph to populate initial state.
    pub fn from_store(store: &KnowledgeStore) -> Result<Self, KnowledgeError> {
        let mut graph = Self::new();

        for (selector_key, leaf_data) in store.leaf_data() {
            let selector_str = selector_key.to_selector_string();
            let leaf = WorkingLeaf {
                selector: selector_str.clone(),
                file_path: leaf_data.file_path.clone(),
                name: leaf_data.name.clone(),
                qualified_name: leaf_data.qualified_name.clone(),
                kind: leaf_data.kind.clone(),
                start_line: leaf_data.start_line,
                end_line: leaf_data.end_line,
                source: leaf_data.source.clone(),
                source_hash: leaf_data.source_hash.clone(),
                parent_qualified_name: leaf_data.parent_qualified_name.clone(),
                children_qualified_names: leaf_data.children_qualified_names.clone(),
            };
            graph
                .file_leaves
                .entry(leaf.file_path.clone())
                .or_default()
                .push(selector_str.clone());
            graph.leaves.insert(selector_str, leaf);
        }

        Ok(graph)
    }

    /// Insert a pre-built working leaf into the graph.
    ///
    /// Used by the tool to populate the graph from a file extraction when
    /// no persisted knowledge store is available.
    pub fn insert_working_leaf(&mut self, selector: String, leaf: WorkingLeaf) {
        self.file_leaves
            .entry(leaf.file_path.clone())
            .or_default()
            .push(selector.clone());
        self.leaves.insert(selector, leaf);
    }

    /// Resolve a selector against the working graph.
    pub fn resolve_leaf(&self, selector: &Selector) -> Option<&WorkingLeaf> {
        let key = selector.to_string();
        self.leaves.get(&key)
    }

    /// Check if a selector resolves to an existing leaf.
    pub fn has_leaf(&self, selector: &Selector) -> bool {
        self.leaves.contains_key(&selector.to_string())
    }

    /// Get all leaves for a given file path.
    pub fn leaves_in_file(&self, file_path: &str) -> Vec<&WorkingLeaf> {
        self.file_leaves
            .get(file_path)
            .map(|selectors| {
                selectors
                    .iter()
                    .filter_map(|s| self.leaves.get(s))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get the version chains for serialization at activity completion.
    pub fn version_chains(&self) -> &HashMap<String, LeafVersionChain> {
        &self.version_chains
    }

    /// Get the source for a leaf (used by knowledge.pack integration).
    pub fn get_leaf_source(&self, selector: &Selector) -> Option<String> {
        self.resolve_leaf(selector).map(|l| l.source.clone())
    }
}
