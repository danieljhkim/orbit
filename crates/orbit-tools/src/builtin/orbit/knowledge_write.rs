use std::path::{Path, PathBuf};

use orbit_knowledge::extract::{self, Language};
use orbit_knowledge::lock::LockStore;
use orbit_knowledge::{
    KnowledgeStore, Selector, WorkingGraph, WorkingLeaf, load_task_working_graph,
    save_task_working_graph,
};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgeWriteTool;

impl Tool for OrbitKnowledgeWriteTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.write".to_string(),
            description: "Edit a symbol or file in the knowledge graph. Accepts symbol selectors (edit/insert leaf) or file selectors (rewrite file or region).".to_string(),
            parameters: vec![
                ToolParam {
                    name: "selector".to_string(),
                    description: "Symbol selector (`symbol:path#symbol:kind`) to edit a leaf, or file selector (`file:path`) for file-level writes.".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "new_source".to_string(),
                    description: "The source code to write".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "position".to_string(),
                    description: "For insert mode: anchor selector like `after:symbol:path#symbol:kind`. Inserts after the anchor leaf.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "start_line".to_string(),
                    description: "For file selectors: start line of region to replace (1-indexed). Requires end_line.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "end_line".to_string(),
                    description: "For file selectors: end line of region to replace (1-indexed, inclusive). Requires start_line.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "reason".to_string(),
                    description: "Optional reason for this edit, stored in the version chain".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "workspace_path".to_string(),
                    description: "Optional workspace root override for branch/worktree targeting".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "knowledge_dir".to_string(),
                    description: "Optional knowledge artifact directory; defaults to `<workspace>/.orbit/knowledge`".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let selector_str = crate::require_str(&input, "selector")?;
        let new_source = require_new_source(&input)?;
        let reason = optional_str(&input, "reason");
        let position_str = optional_str(&input, "position");

        let selector: Selector = selector_str
            .parse()
            .map_err(|e| OrbitError::InvalidInput(format!("{e}")))?;

        // Reject dir selectors — only symbol and file are valid
        if matches!(selector, Selector::Dir { .. }) {
            return Err(OrbitError::InvalidInput(
                "graph.write does not accept dir selectors".to_string(),
            ));
        }

        let workspace_root_buf = resolve_workspace_root_with_override(ctx, &input)?;
        let workspace_root = workspace_root_buf.as_path();

        let knowledge_dir = resolve_knowledge_dir(ctx, &input)?;
        let mut working_graph =
            match load_task_working_graph(ctx.orbit_root.as_deref(), ctx.task_id.as_deref())? {
                Some(graph) => graph,
                None => initialize_working_graph(&knowledge_dir, &selector, workspace_root)?,
            };

        let lock_owner = ctx
            .agent_name
            .as_deref()
            .or(ctx.task_id.as_deref())
            .unwrap_or("unknown");
        let position_selector = parse_position_selector(position_str.as_deref())?;
        let lock_targets = lock_targets_for_mutation(&selector, &[]);

        let result = with_graph_locks(
            &knowledge_dir,
            lock_owner,
            ctx.task_id.as_deref(),
            reason.as_deref().unwrap_or("editing"),
            &lock_targets,
            || {
                let result = match &selector {
                    Selector::File { path } => {
                        let start_line = input.get("start_line").and_then(Value::as_u64);
                        let end_line = input.get("end_line").and_then(Value::as_u64);
                        match (start_line, end_line) {
                            (Some(sl), Some(el)) => working_graph
                                .rewrite_file_region(
                                    path,
                                    sl as usize,
                                    el as usize,
                                    &new_source,
                                    reason.as_deref(),
                                    workspace_root,
                                )
                                .map_err(write_err_to_orbit)?,
                            (None, None) => working_graph
                                .rewrite_file(path, &new_source, reason.as_deref(), workspace_root)
                                .map_err(write_err_to_orbit)?,
                            _ => {
                                return Err(OrbitError::InvalidInput(
                                    "both start_line and end_line are required for region edits"
                                        .to_string(),
                                ));
                            }
                        }
                    }
                    Selector::Symbol { .. } => {
                        if working_graph.has_leaf(&selector) {
                            working_graph
                                .edit_leaf(
                                    &selector,
                                    &new_source,
                                    reason.as_deref(),
                                    workspace_root,
                                )
                                .map_err(write_err_to_orbit)?
                        } else {
                            working_graph
                                .insert_leaf(
                                    &selector,
                                    &new_source,
                                    position_selector.as_ref(),
                                    reason.as_deref(),
                                    workspace_root,
                                )
                                .map_err(write_err_to_orbit)?
                        }
                    }
                    Selector::Dir { .. } => unreachable!(),
                };

                save_task_working_graph(
                    ctx.orbit_root.as_deref(),
                    ctx.task_id.as_deref(),
                    &working_graph,
                )?;
                Ok(result)
            },
        )?;

        serde_json::to_value(result)
            .map_err(|e| OrbitError::Execution(format!("serialize result: {e}")))
    }
}

/// Resolve the effective workspace root, with optional override from input.
pub(super) fn resolve_workspace_root_with_override(
    ctx: &ToolContext,
    input: &Value,
) -> Result<std::path::PathBuf, OrbitError> {
    if let Some(ws) = input.get("workspace_path").and_then(Value::as_str)
        && !ws.trim().is_empty()
    {
        return Ok(std::path::PathBuf::from(ws));
    }
    ctx.workspace_root
        .clone()
        .ok_or_else(|| OrbitError::InvalidInput("workspace_root is required".to_string()))
}

pub(super) fn write_err_to_orbit(e: orbit_knowledge::WriteError) -> OrbitError {
    serde_json::to_value(&e)
        .map(|v| OrbitError::Execution(v.to_string()))
        .unwrap_or_else(|_| OrbitError::Execution(format!("{:?}", e)))
}

pub(super) fn lock_targets_for_mutation(selector: &Selector, extra_files: &[&str]) -> Vec<String> {
    let mut targets = Vec::new();
    match selector {
        Selector::File { path } => targets.push(format!("file:{path}")),
        Selector::Symbol { path, .. } => {
            targets.push(selector.to_string());
            targets.push(format!("file:{path}"));
        }
        Selector::Dir { .. } => {}
    }

    for file in extra_files {
        let file_selector = format!("file:{file}");
        if !targets.contains(&file_selector) {
            targets.push(file_selector);
        }
    }

    targets
}

pub(super) fn with_graph_locks<T, F>(
    knowledge_dir: &Path,
    owner: &str,
    task_id: Option<&str>,
    reason: &str,
    selectors: &[String],
    f: F,
) -> Result<T, OrbitError>
where
    F: FnOnce() -> Result<T, OrbitError>,
{
    let lock_path = orbit_knowledge::lock::lock_store_path(knowledge_dir);
    let mut store = LockStore::load(&lock_path)
        .map_err(|e| OrbitError::Execution(format!("load graph locks: {e}")))?;
    for selector in selectors {
        store
            .lock(selector, owner, task_id, reason)
            .map_err(|e| OrbitError::Execution(format!("lock failed: {e}")))?;
    }
    store
        .save(&lock_path)
        .map_err(|e| OrbitError::Execution(format!("save graph locks: {e}")))?;

    let operation_result = f();

    let unlock_result = (|| -> Result<(), OrbitError> {
        let mut store = LockStore::load(&lock_path)
            .map_err(|e| OrbitError::Execution(format!("reload graph locks: {e}")))?;
        for selector in selectors {
            store
                .unlock(selector, owner)
                .map_err(|e| OrbitError::Execution(format!("unlock failed: {e}")))?;
        }
        store
            .save(&lock_path)
            .map_err(|e| OrbitError::Execution(format!("save graph locks: {e}")))
    })();

    match (operation_result, unlock_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(unlock_error)) => Err(unlock_error),
        (Err(error), Err(unlock_error)) => Err(OrbitError::Execution(format!(
            "{error}; also failed to release graph locks: {unlock_error}"
        ))),
    }
}

fn require_new_source(input: &Value) -> Result<String, OrbitError> {
    let value = input
        .get("new_source")
        .ok_or_else(|| OrbitError::InvalidInput("missing `new_source`".to_string()))?;
    let raw = value
        .as_str()
        .ok_or_else(|| OrbitError::InvalidInput("`new_source` must be a string".to_string()))?;
    if raw.trim().is_empty() {
        return Err(OrbitError::InvalidInput(
            "`new_source` must not be empty".to_string(),
        ));
    }
    Ok(raw.to_string())
}

fn optional_str(input: &Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
}

fn parse_position_selector(position: Option<&str>) -> Result<Option<Selector>, OrbitError> {
    let Some(pos) = position else {
        return Ok(None);
    };

    // Strip "after:" prefix if present
    let selector_str = pos.strip_prefix("after:").unwrap_or(pos);
    let selector: Selector = selector_str
        .parse()
        .map_err(|e| OrbitError::InvalidInput(format!("invalid position: {e}")))?;
    Ok(Some(selector))
}

pub(super) fn resolve_knowledge_dir(
    ctx: &ToolContext,
    input: &Value,
) -> Result<PathBuf, OrbitError> {
    if let Some(raw) = input.get("knowledge_dir").and_then(|v| v.as_str())
        && !raw.trim().is_empty()
    {
        let path = Path::new(raw);
        if path.is_absolute() {
            return Ok(path.to_path_buf());
        }
        if let Some(ws) = ctx.workspace_root.as_deref() {
            return Ok(ws.join(path));
        }
        return Ok(path.to_path_buf());
    }

    // Prefer orbit_root (the resolved .orbit data directory) over workspace_root.
    // In worktree contexts, workspace_root points to the worktree checkout while
    // orbit_root points to the main repo's .orbit/ where knowledge artifacts live.
    if let Some(orbit_root) = ctx.orbit_root.as_deref() {
        return Ok(orbit_root.join("knowledge"));
    }

    let Some(workspace_root) = ctx.workspace_root.as_deref() else {
        return Err(OrbitError::InvalidInput(
            "`knowledge_dir` is required when `workspace_root` is unavailable".to_string(),
        ));
    };
    Ok(workspace_root.join(".orbit/knowledge"))
}

/// Initialize a working graph by loading the persisted knowledge store,
/// or create a minimal graph by extracting the target file.
pub(super) fn initialize_working_graph(
    knowledge_dir: &Path,
    selector: &Selector,
    workspace_root: &Path,
) -> Result<WorkingGraph, OrbitError> {
    // Try to load from persisted store
    if let Ok(store) = KnowledgeStore::open(knowledge_dir)
        && let Ok(mut graph) = WorkingGraph::from_store(&store)
    {
        graph.seed_file_snapshots_from_workspace(workspace_root);
        return Ok(graph);
    }

    // Fallback: extract the target file to build a minimal working graph
    let Selector::Symbol { path, .. } = selector else {
        return Ok(WorkingGraph::new());
    };

    let file_path = workspace_root.join(path);
    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let Some(language) = Language::from_extension(ext) else {
        return Ok(WorkingGraph::new());
    };

    let content = std::fs::read_to_string(&file_path)
        .map_err(|e| OrbitError::Execution(format!("read {}: {e}", file_path.display())))?;

    let extraction = extract::extract_file(&content, language);
    let mut graph = WorkingGraph::new();

    // Populate from extraction
    for leaf in &extraction.leaves {
        let sel_str = format!("symbol:{path}#{}:{}", leaf.qualified_name, leaf.kind);
        let working_leaf = WorkingLeaf {
            selector: sel_str.clone(),
            file_path: path.clone(),
            name: leaf.name.clone(),
            qualified_name: leaf.qualified_name.clone(),
            kind: leaf.kind.clone(),
            start_line: leaf.start_line,
            end_line: leaf.end_line,
            source: leaf.source.clone(),
            source_hash: leaf.source_hash.clone(),
            parent_qualified_name: leaf.parent_qualified_name.clone(),
            children_qualified_names: leaf.children_qualified_names.clone(),
        };
        graph.insert_working_leaf(sel_str, working_leaf);
    }
    graph.seed_file_snapshots_from_workspace(workspace_root);

    Ok(graph)
}
