use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::error::KnowledgeError;
use crate::graph::object_store::GraphReadOptions;
use orbit_graph_extract::Selector;
use orbit_graph_extract::selector::SelectorLookupKey;

use super::KnowledgeStore;
use super::graph_io::{extract_leaf_source, read_graph_object};
use super::types::{
    KnowledgeEntryKind, KnowledgePack, KnowledgePackEntry, KnowledgePackTimeout,
    UnresolvedSelectorReason,
};

const FILE_SOURCE_HINT: &str = "File selectors return metadata only. Use `orbit.graph.show` or `symbol:` selectors when you need source.";
const SELECTOR_TIMEOUT_HINT: &str = "Selector packing timed out before this selector was resolved; retry with a larger `timeout_ms` or read this selector directly.";
const OUTSIDE_INDEXED_ROOTS_HINT: &str = "This file exists on disk but is outside the graph's indexed roots; read it directly or add it to an indexed path before packing it.";
const NOT_FOUND_HINT: &str = "This file does not exist on disk in the current workspace.";
const STALE_SNAPSHOT_HINT: &str = "This file exists under an indexed root but is absent from the current graph snapshot; run `orbit graph build --refresh` and retry.";

impl KnowledgeStore {
    pub fn pack(&self, selectors: &[Selector]) -> Result<KnowledgePack, KnowledgeError> {
        self.pack_with_timeout(selectors, None)
    }

    pub fn pack_with_timeout(
        &self,
        selectors: &[Selector],
        timeout_ms: Option<u64>,
    ) -> Result<KnowledgePack, KnowledgeError> {
        self.pack_with_timeout_options(
            selectors,
            timeout_ms,
            GraphReadOptions {
                hydrate_leaf_source: true,
                ..Default::default()
            },
            None,
        )
    }

    pub fn pack_with_timeout_options(
        &self,
        selectors: &[Selector],
        timeout_ms: Option<u64>,
        read_options: GraphReadOptions,
        workspace_root: Option<&Path>,
    ) -> Result<KnowledgePack, KnowledgeError> {
        let mut entries = Vec::with_capacity(selectors.len());
        let mut unresolved_selectors = Vec::new();
        let timeout = timeout_ms.map(Duration::from_millis);
        let started_at = Instant::now();
        let mut timed_out_after = None;

        for (index, selector) in selectors.iter().enumerate() {
            if timeout.is_some_and(|timeout| started_at.elapsed() >= timeout) {
                timed_out_after = Some(index);
                for remaining in &selectors[index..] {
                    let selector_string = remaining.to_string();
                    unresolved_selectors.push(selector_string.clone());
                    let mut entry = unresolved_entry(selector_string);
                    entry.hint = Some(SELECTOR_TIMEOUT_HINT.to_string());
                    entries.push(entry);
                }
                break;
            }

            let selector_string = selector.to_string();
            let Some(node_id) = self.selector_index.get(&selector.lookup_key()).cloned() else {
                unresolved_selectors.push(selector_string.clone());
                let mut entry = unresolved_entry(selector_string);
                self.apply_unresolved_file_diagnostic(selector, workspace_root, &mut entry);
                entries.push(entry);
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
            let source = if index_entry.node_type == "leaf" && read_options.hydrate_leaf_source {
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
        let timeout = timed_out_after.map(|processed_selectors| KnowledgePackTimeout {
            timeout_ms: timeout_ms.unwrap_or_default(),
            processed_selectors,
            total_selectors: selectors.len(),
            hint: SELECTOR_TIMEOUT_HINT.to_string(),
        });
        Ok(KnowledgePack {
            knowledge_dir: self.knowledge_dir.display().to_string(),
            manifest_generated_at: self.manifest.generated_at.clone(),
            unresolved_selectors,
            timeout,
            total_nodes,
            entries,
        })
    }

    fn apply_unresolved_file_diagnostic(
        &self,
        selector: &Selector,
        workspace_root: Option<&Path>,
        entry: &mut KnowledgePackEntry,
    ) {
        let Some((reason, hint)) = self.unresolved_file_diagnostic(selector, workspace_root) else {
            return;
        };
        entry.reason = Some(reason);
        entry.hint = Some(hint.to_string());
    }

    fn unresolved_file_diagnostic(
        &self,
        selector: &Selector,
        workspace_root: Option<&Path>,
    ) -> Option<(UnresolvedSelectorReason, &'static str)> {
        let Selector::File { path } = selector else {
            return None;
        };
        let workspace_root = workspace_root?;
        if !selector_path_on_disk(workspace_root, path).exists() {
            return Some((UnresolvedSelectorReason::NotFound, NOT_FOUND_HINT));
        }
        if path_has_hidden_component(path) || !self.path_has_indexed_ancestor(path) {
            return Some((
                UnresolvedSelectorReason::OutsideIndexedRoots,
                OUTSIDE_INDEXED_ROOTS_HINT,
            ));
        }
        Some((UnresolvedSelectorReason::StaleSnapshot, STALE_SNAPSHOT_HINT))
    }

    fn path_has_indexed_ancestor(&self, path: &str) -> bool {
        let mut candidate = selector_parent_dir(path);
        loop {
            if self
                .selector_index
                .contains_key(&SelectorLookupKey::Dir(candidate.clone()))
            {
                return true;
            }
            if candidate == "." {
                return false;
            }
            candidate = selector_parent_dir(&candidate);
        }
    }
}

pub(super) fn unresolved_entry(selector: String) -> KnowledgePackEntry {
    KnowledgePackEntry::unresolved(selector)
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
        file: None,
        kind,
        reason: None,
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

fn selector_path_on_disk(workspace_root: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    }
}

fn path_has_hidden_component(path: &str) -> bool {
    Path::new(path).components().any(|component| {
        let Component::Normal(segment) = component else {
            return false;
        };
        segment
            .to_str()
            .is_some_and(|segment| segment.starts_with('.'))
    })
}

fn selector_parent_dir(path: &str) -> String {
    let parent = Path::new(path).parent().unwrap_or_else(|| Path::new("."));
    if parent.as_os_str().is_empty() {
        ".".to_string()
    } else {
        parent.to_string_lossy().replace('\\', "/")
    }
}
