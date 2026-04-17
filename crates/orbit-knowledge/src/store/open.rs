use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use crate::error::KnowledgeError;
use crate::selector::SelectorLookupKey;

use super::KnowledgeStore;
use super::graph_io::{CurrentRefFile, read_json_file};
use super::types::SymbolSummary;

impl KnowledgeStore {
    pub fn open(knowledge_dir: &Path) -> Result<Self, KnowledgeError> {
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

        let current_ref_path = knowledge_dir.join("graph/refs/current.json");
        let current_ref: CurrentRefFile = read_json_file(&current_ref_path).map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!(
                "graph reference is unavailable or invalid at {}: {error}",
                current_ref_path.display()
            ))
        })?;

        let graph_index_path = knowledge_dir.join(&current_ref.index);
        let graph_index = read_json_file(&graph_index_path).map_err(|error| {
            KnowledgeError::knowledge_unavailable(format!(
                "graph index is unavailable or invalid at {}: {error}",
                graph_index_path.display()
            ))
        })?;

        let selector_index = build_selector_index(&graph_index)?;

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

    pub(super) fn dir_child_selectors(&self, dir_node_id: &str) -> Option<Vec<String>> {
        let dir_location = self
            .graph_index
            .nodes
            .get(dir_node_id)?
            .location
            .trim_end_matches('/')
            .to_string();

        let mut children = Vec::new();
        for entry in self.graph_index.nodes.values() {
            let entry_parent = match entry.node_type.as_str() {
                "dir" => {
                    let loc = entry.location.trim_end_matches('/');
                    std::path::Path::new(loc)
                        .parent()
                        .map(|path| path.to_string_lossy().into_owned())
                }
                "file" => std::path::Path::new(&entry.location)
                    .parent()
                    .map(|path| path.to_string_lossy().into_owned()),
                _ => continue,
            };
            let parent = match entry_parent {
                Some(parent)
                    if parent == dir_location || (parent.is_empty() && dir_location == ".") =>
                {
                    parent
                }
                _ => continue,
            };
            let _ = parent;

            let selector = match entry.node_type.as_str() {
                "dir" => format!("dir:{}", entry.location.trim_end_matches('/')),
                "file" => format!("file:{}", entry.location),
                _ => continue,
            };
            children.push(selector);
        }

        children.sort();
        (!children.is_empty()).then_some(children)
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
