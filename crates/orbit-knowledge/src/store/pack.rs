use serde_json::Value;

use crate::error::KnowledgeError;
use crate::selector::Selector;

use super::KnowledgeStore;
use super::graph_io::{extract_leaf_source, read_graph_object};
use super::types::{KnowledgeEntryKind, KnowledgePack, KnowledgePackEntry};

const FILE_SOURCE_HINT: &str = "File selectors return metadata only. Use `orbit.graph.show` or `symbol:` selectors when you need source.";

impl KnowledgeStore {
    pub fn pack(&self, selectors: &[Selector]) -> Result<KnowledgePack, KnowledgeError> {
        let mut entries = Vec::with_capacity(selectors.len());
        let mut unresolved_selectors = Vec::new();

        for selector in selectors {
            let selector_string = selector.to_string();
            let Some(node_id) = self.selector_index.get(&selector.lookup_key()).cloned() else {
                unresolved_selectors.push(selector_string.clone());
                entries.push(unresolved_entry(selector_string));
                continue;
            };

            let index_entry = self.graph_index.nodes.get(&node_id).ok_or_else(|| {
                KnowledgeError::invalid_data(format!(
                    "graph index entry disappeared for `{node_id}`"
                ))
            })?;

            let object = read_graph_object(
                &self.knowledge_dir,
                &index_entry.object_hash,
                self.graph_object_cache(),
            )?;
            let node = object.get("node");
            let source = if index_entry.node_type == "leaf" {
                extract_leaf_source(&self.knowledge_dir, &object, self.graph_object_cache())?
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

            let child_selectors = if kind == KnowledgeEntryKind::Dir {
                self.dir_child_selectors(&node_id)
            } else {
                None
            };

            let mut entry = project_entry(selector_string, kind, node, source, child_selectors);
            if entry.kind == KnowledgeEntryKind::File {
                entry.symbol_summary = self.file_symbol_summary(node);
                entry.hint = Some(FILE_SOURCE_HINT.to_string());
            }
            entries.push(entry);
        }

        let total_nodes = entries.iter().filter(|entry| entry.resolved).count();
        Ok(KnowledgePack {
            knowledge_dir: self.knowledge_dir.display().to_string(),
            manifest_generated_at: self.manifest.generated_at.clone(),
            unresolved_selectors,
            total_nodes,
            entries,
        })
    }
}

pub(super) fn unresolved_entry(selector: String) -> KnowledgePackEntry {
    KnowledgePackEntry {
        selector,
        kind: KnowledgeEntryKind::Unresolved,
        name: None,
        language: None,
        description: None,
        extension: None,
        imports: None,
        exports: None,
        re_exports: None,
        children: None,
        symbol_summary: None,
        source: None,
        hint: None,
        start_line: None,
        end_line: None,
        input_signature: None,
        output_signature: None,
        resolved: false,
    }
}

fn project_entry(
    selector: String,
    kind: KnowledgeEntryKind,
    node: Option<&Value>,
    source: Option<String>,
    child_selectors: Option<Vec<String>>,
) -> KnowledgePackEntry {
    let str_field = |key| {
        node.and_then(|entry| entry.get(key))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(String::from)
    };
    let str_vec_field = |key| -> Option<Vec<String>> {
        node.and_then(|entry| entry.get(key))
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(String::from))
                    .collect()
            })
    };
    let val_vec_field = |key| -> Option<Vec<Value>> {
        node.and_then(|entry| entry.get(key))
            .and_then(Value::as_array)
            .filter(|values| !values.is_empty())
            .cloned()
    };
    let u32_field = |key| {
        node.and_then(|entry| entry.get(key))
            .and_then(Value::as_u64)
            .map(|value| value as u32)
    };

    KnowledgePackEntry {
        selector,
        kind,
        name: str_field("name"),
        language: str_field("language"),
        description: str_field("description"),
        extension: str_field("extension"),
        imports: str_vec_field("imports"),
        exports: str_vec_field("exports"),
        re_exports: val_vec_field("re_exports"),
        children: child_selectors,
        symbol_summary: None,
        source,
        hint: None,
        start_line: u32_field("start_line"),
        end_line: u32_field("end_line"),
        input_signature: val_vec_field("input_signature"),
        output_signature: val_vec_field("output_signature"),
        resolved: true,
    }
}
