use serde_json::Value;

use orbit_graph_extract::selector::SelectorLookupKey;

use super::KnowledgeStore;
use super::graph_io::{extract_leaf_source, read_graph_object};
use super::types::LeafData;

impl KnowledgeStore {
    pub(crate) fn leaf_data(&self) -> Vec<(SelectorLookupKey, LeafData)> {
        let mut result = Vec::new();

        for entry in self.graph_index.nodes.values() {
            if entry.node_type != "leaf" {
                continue;
            }
            let kind = match &entry.kind {
                Some(kind) => kind.clone(),
                None => continue,
            };

            let object = match read_graph_object(
                &self.knowledge_dir,
                &entry.object_hash,
                self.graph_object_cache(),
            ) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let Some(node) = object.get("node") else {
                continue;
            };

            let start_line = node.get("start_line").and_then(Value::as_u64).unwrap_or(0) as usize;
            let end_line = node.get("end_line").and_then(Value::as_u64).unwrap_or(0) as usize;
            let name = node
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let source_hash = node
                .get("source_hash")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let source =
                extract_leaf_source(&self.knowledge_dir, &object, self.graph_object_cache())
                    .ok()
                    .flatten()
                    .unwrap_or_default();

            let Some((file_path, qualified_name)) = entry.location.split_once('#') else {
                continue;
            };

            let children_qualified_names = node
                .get("children")
                .and_then(Value::as_array)
                .map(|children| {
                    children
                        .iter()
                        .filter_map(|value| value.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            result.push((
                SelectorLookupKey::Symbol(entry.location.clone(), kind.clone()),
                LeafData {
                    file_path: file_path.to_string(),
                    name,
                    qualified_name: qualified_name.to_string(),
                    kind,
                    start_line,
                    end_line,
                    source,
                    source_hash,
                    parent_qualified_name: None,
                    children_qualified_names,
                },
            ));
        }

        result
    }
}
