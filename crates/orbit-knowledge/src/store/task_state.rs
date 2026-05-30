use std::fs;
use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;

use crate::io::write_text_atomic_durable;
use crate::working_graph::{WorkingGraph, WorkingLeaf};
use orbit_graph_extract::Selector;

use super::pack::unresolved_entry;
use super::types::{KnowledgeEntryKind, KnowledgePack, KnowledgePackEntry};

const TASK_WORKING_GRAPH_DIR: &str = "state/worktrees/knowledge";

pub fn task_working_graph_state_path(
    orbit_root: Option<&Path>,
    task_id: Option<&str>,
) -> Option<PathBuf> {
    task_graph_path_in_dir(orbit_root?, task_id?, TASK_WORKING_GRAPH_DIR)
}

pub fn load_task_working_graph(
    orbit_root: Option<&Path>,
    task_id: Option<&str>,
) -> Result<Option<WorkingGraph>, OrbitError> {
    let Some(path) = task_working_graph_state_path(orbit_root, task_id) else {
        return Ok(None);
    };
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(&path)
        .map_err(|error| OrbitError::Execution(format!("read {}: {error}", path.display())))?;
    let graph = serde_json::from_str(&raw).map_err(|error| {
        OrbitError::Execution(format!(
            "parse task working graph state {}: {error}",
            path.display()
        ))
    })?;
    Ok(Some(graph))
}

pub fn save_task_working_graph(
    orbit_root: Option<&Path>,
    task_id: Option<&str>,
    graph: &WorkingGraph,
) -> Result<(), OrbitError> {
    let Some(path) = task_working_graph_state_path(orbit_root, task_id) else {
        return Ok(());
    };

    let parent = path
        .parent()
        .ok_or_else(|| OrbitError::Execution(format!("no parent dir for {}", path.display())))?;
    fs::create_dir_all(parent)
        .map_err(|error| OrbitError::Execution(format!("create {}: {error}", parent.display())))?;

    let payload = serde_json::to_string_pretty(graph).map_err(|error| {
        OrbitError::Execution(format!(
            "serialize task working graph state {}: {error}",
            path.display()
        ))
    })?;
    write_text_atomic_durable(&path, &format!("{payload}\n"))
        .map_err(|error| OrbitError::Execution(format!("write {}: {error}", path.display())))?;
    Ok(())
}

pub fn overlay_pack_with_working_graph(
    mut pack: KnowledgePack,
    selectors: &[Selector],
    graph: &WorkingGraph,
) -> KnowledgePack {
    for (entry, selector) in pack.entries.iter_mut().zip(selectors.iter()) {
        let Some(leaf) = graph.resolve_leaf(selector) else {
            continue;
        };
        apply_working_leaf(entry, leaf);
    }

    pack.unresolved_selectors = pack
        .entries
        .iter()
        .filter(|entry| !entry.resolved)
        .map(|entry| entry.selector.clone())
        .collect();
    pack.total_nodes = pack.entries.iter().filter(|entry| entry.resolved).count();
    pack
}

pub fn pack_from_working_graph(
    knowledge_dir: &Path,
    selectors: &[Selector],
    graph: &WorkingGraph,
) -> KnowledgePack {
    let mut entries = Vec::with_capacity(selectors.len());

    for selector in selectors {
        let selector_string = selector.to_string();
        if let Some(leaf) = graph.resolve_leaf(selector) {
            let mut entry = unresolved_entry(selector_string);
            apply_working_leaf(&mut entry, leaf);
            entries.push(entry);
        } else {
            entries.push(unresolved_entry(selector_string));
        }
    }

    let unresolved_selectors = entries
        .iter()
        .filter(|entry| !entry.resolved)
        .map(|entry| entry.selector.clone())
        .collect();
    let total_nodes = entries.iter().filter(|entry| entry.resolved).count();

    KnowledgePack {
        knowledge_dir: knowledge_dir.display().to_string(),
        manifest_generated_at: String::new(),
        unresolved_selectors,
        timeout: None,
        total_nodes,
        entries,
    }
}

fn task_graph_path_in_dir(orbit_root: &Path, task_id: &str, dir: &str) -> Option<PathBuf> {
    let task_id = task_id.trim();
    if task_id.is_empty() {
        return None;
    }
    Some(orbit_root.join(dir).join(format!("{task_id}.json")))
}

fn apply_working_leaf(entry: &mut KnowledgePackEntry, leaf: &WorkingLeaf) {
    entry.kind = KnowledgeEntryKind::Leaf;
    entry.resolved = true;
    entry.name = Some(leaf.name.clone());
    entry.source = Some(leaf.source.clone());
    entry.start_line = Some(leaf.start_line as u32);
    entry.end_line = Some(leaf.end_line as u32);
    entry.language = Path::new(&leaf.file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(String::from);
}
