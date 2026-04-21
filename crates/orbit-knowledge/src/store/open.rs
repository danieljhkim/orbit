use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use crate::error::KnowledgeError;
use crate::graph::object_store::{GraphObjectStore, RefName};
use crate::selector::SelectorLookupKey;

use super::KnowledgeStore;
use super::graph_io::read_json_file;
use super::types::SymbolSummary;

impl KnowledgeStore {
    pub fn open(
        knowledge_dir: &Path,
        requested_ref: &RefName,
        fallback_ref: Option<&RefName>,
        default_ref: Option<&RefName>,
    ) -> Result<Self, KnowledgeError> {
        if !knowledge_dir.is_dir() {
            return Err(KnowledgeError::knowledge_unavailable(format!(
                "knowledge directory does not exist: {}",
                knowledge_dir.display()
            )));
        }

        let manifest_path = knowledge_dir.join("manifest.json");
        let manifest = read_json_file(&manifest_path).map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!(
                "manifest.json is unavailable or invalid at {}: {error}",
                manifest_path.display()
            ))
        })?;

        let graph_store = GraphObjectStore::new(knowledge_dir.join("graph"));
        graph_store.prepare_refs_layout(default_ref)?;
        let resolved_ref = graph_store.resolve_ref(requested_ref, fallback_ref)?;
        let graph_index_path = resolved_ref.index_path;
        let graph_index = read_json_file(&graph_index_path).map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!(
                "graph index is unavailable or invalid at {}: {error}",
                graph_index_path.display()
            ))
        })?;

        let selector_index = build_selector_index(&graph_index)?;
        let dir_children_index = build_dir_children_index(&graph_index);

        Ok(Self {
            knowledge_dir: knowledge_dir.to_path_buf(),
            manifest,
            graph_index,
            selector_index,
            dir_children_index,
        })
    }

    pub fn is_available(
        knowledge_dir: &Path,
        requested_ref: &RefName,
        fallback_ref: Option<&RefName>,
        default_ref: Option<&RefName>,
    ) -> bool {
        Self::open(knowledge_dir, requested_ref, fallback_ref, default_ref).is_ok()
    }

    pub(super) fn dir_child_selectors(&self, dir_node_id: &str) -> Option<Vec<String>> {
        let dir_location = self
            .graph_index
            .nodes
            .get(dir_node_id)?
            .location
            .trim_end_matches('/')
            .to_string();
        self.dir_children_index.get(&dir_location).cloned()
    }

    pub(super) fn file_symbol_summary(&self, node: Option<&Value>) -> Option<Vec<SymbolSummary>> {
        let leaf_ids = node?.get("leaf_children").and_then(Value::as_array)?;
        let mut summaries = Vec::new();
        for leaf_id_val in leaf_ids {
            let leaf_id = leaf_id_val.as_str()?;
            let entry = self.graph_index.nodes.get(leaf_id)?;
            let kind = entry.kind.clone().unwrap_or_default();
            let (location, _) = entry
                .location
                .rsplit_once(':')
                .unwrap_or((&entry.location, ""));
            let name = location
                .rsplit_once('#')
                .map(|(_, name)| name.to_string())
                .unwrap_or_default();
            summaries.push(SymbolSummary {
                selector: format!("symbol:{}:{}", entry.location, kind),
                name,
                kind,
            });
        }

        (!summaries.is_empty()).then_some(summaries)
    }
}

fn build_dir_children_index(
    graph_index: &super::graph_io::GraphIndexFile,
) -> HashMap<String, Vec<String>> {
    let mut index = HashMap::<String, Vec<String>>::new();

    for entry in graph_index.nodes.values() {
        let selector = match entry.node_type.as_str() {
            "dir" => {
                let location = entry.location.trim_end_matches('/');
                if location.is_empty() || location == "." {
                    continue;
                }
                format!("dir:{location}")
            }
            "file" => format!("file:{}", entry.location),
            _ => continue,
        };

        let parent = match entry.node_type.as_str() {
            "dir" => std::path::Path::new(entry.location.trim_end_matches('/'))
                .parent()
                .map(|path| path.to_string_lossy().into_owned()),
            "file" => std::path::Path::new(&entry.location)
                .parent()
                .map(|path| path.to_string_lossy().into_owned()),
            _ => None,
        }
        .filter(|parent| !parent.is_empty())
        .unwrap_or_else(|| ".".to_string());

        index.entry(parent).or_default().push(selector);
    }

    for children in index.values_mut() {
        children.sort();
    }

    index
}

fn build_selector_index(
    graph_index: &super::graph_io::GraphIndexFile,
) -> Result<HashMap<SelectorLookupKey, String>, KnowledgeError> {
    let mut selector_index = HashMap::new();

    for (node_id, entry) in &graph_index.nodes {
        let key = match entry.node_type.as_str() {
            "dir" => SelectorLookupKey::Dir(entry.location.trim_end_matches('/').to_string()),
            "file" => SelectorLookupKey::File(entry.location.clone()),
            "leaf" => SelectorLookupKey::Symbol(
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

    Ok(selector_index)
}
