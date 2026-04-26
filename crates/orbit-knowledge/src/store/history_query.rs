//! Task-ID query helpers for `KnowledgeStore` (T20260421-0528).
//!
//! Reads the on-disk node object for a given selector and extracts the
//! `task_ids` + `structural_conflict` fields. Used by
//! [`crate::service::history`] to answer `orbit task history <selector>`
//! without re-parsing the full graph.

use serde_json::Value;

use crate::error::KnowledgeError;
use crate::selector::Selector;

use super::KnowledgeStore;
use super::graph_io::read_graph_object;

/// Node-level task-attribution data returned for a resolved selector.
#[derive(Debug, Clone)]
pub struct NodeTaskInfo {
    pub node_id: String,
    pub node_type: String,
    pub location: String,
    pub task_ids: Vec<String>,
    pub structural_conflict: bool,
}

impl KnowledgeStore {
    /// Resolve the selector to a node and read its `task_ids` +
    /// `structural_conflict` from the on-disk node object. Returns `None` when
    /// the selector does not match any node in the loaded graph index.
    pub fn node_task_info(
        &self,
        selector: &Selector,
    ) -> Result<Option<NodeTaskInfo>, KnowledgeError> {
        let Some(node_id) = self.selector_index.get(&selector.lookup_key()).cloned() else {
            return Ok(None);
        };
        let index_entry = self.graph_index.nodes.get(&node_id).ok_or_else(|| {
            KnowledgeError::invalid_data(format!("graph index entry disappeared for `{node_id}`"))
        })?;

        let object = read_graph_object(
            &self.knowledge_dir,
            &index_entry.object_hash,
            self.graph_object_cache(),
        )?;
        let node_value = object.get("node").cloned().unwrap_or(Value::Null);

        let task_ids = node_value
            .get("task_ids")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default();

        let structural_conflict = node_value
            .get("structural_conflict")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        Ok(Some(NodeTaskInfo {
            node_id,
            node_type: index_entry.node_type.clone(),
            location: index_entry.location.clone(),
            task_ids,
            structural_conflict,
        }))
    }
}
