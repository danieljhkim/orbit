use std::path::{Path, PathBuf};

use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::knowledge::Selector;
use crate::knowledge::working_graph::WorkingGraph;
use crate::{Tool, ToolContext};

pub struct OrbitKnowledgeWriteTool;

impl Tool for OrbitKnowledgeWriteTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.knowledge.write".to_string(),
            description: "Edit or insert a leaf in the knowledge graph, writing to disk and updating the working graph".to_string(),
            parameters: vec![
                ToolParam {
                    name: "selector".to_string(),
                    description: "Leaf selector like `leaf:path#symbol:kind`. If it resolves to an existing leaf, the tool edits it. If not, it inserts a new leaf.".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "new_source".to_string(),
                    description: "The source code to write for this leaf".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "position".to_string(),
                    description: "For insert mode: anchor selector like `after:leaf:path#symbol:kind`. Inserts after the anchor leaf. Omit to append before `#[cfg(test)]` or at end of file.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "reason".to_string(),
                    description: "Optional reason for this edit, stored in the version chain".to_string(),
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

        // Only leaf selectors are valid for knowledge.write
        if !matches!(selector, Selector::Leaf { .. }) {
            return Err(OrbitError::InvalidInput(
                "knowledge.write requires a leaf selector (leaf:path#symbol:kind)".to_string(),
            ));
        }

        let workspace_root = ctx
            .workspace_root
            .as_deref()
            .ok_or_else(|| OrbitError::InvalidInput("workspace_root is required".to_string()))?;

        // Initialize or retrieve working graph from context
        // For now, we create a fresh working graph per call and populate it
        // from the file on disk. In a full integration, the working graph
        // would be stored in the ToolContext and shared across calls.
        let knowledge_dir = resolve_knowledge_dir(ctx, &input)?;
        let mut working_graph =
            initialize_working_graph(&knowledge_dir, &selector, workspace_root)?;

        if working_graph.has_leaf(&selector) {
            // Edit mode
            let result = working_graph
                .edit_leaf(&selector, &new_source, reason.as_deref(), workspace_root)
                .map_err(|e| {
                    serde_json::to_value(&e)
                        .map(|v| OrbitError::Execution(v.to_string()))
                        .unwrap_or_else(|_| OrbitError::Execution(format!("{:?}", e)))
                })?;
            serde_json::to_value(result)
                .map_err(|e| OrbitError::Execution(format!("serialize result: {e}")))
        } else {
            // Insert mode
            let position_selector = parse_position_selector(position_str.as_deref())?;
            let result = working_graph
                .insert_leaf(
                    &selector,
                    &new_source,
                    position_selector.as_ref(),
                    reason.as_deref(),
                    workspace_root,
                )
                .map_err(|e| {
                    serde_json::to_value(&e)
                        .map(|v| OrbitError::Execution(v.to_string()))
                        .unwrap_or_else(|_| OrbitError::Execution(format!("{:?}", e)))
                })?;
            serde_json::to_value(result)
                .map_err(|e| OrbitError::Execution(format!("serialize result: {e}")))
        }
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

fn resolve_knowledge_dir(ctx: &ToolContext, input: &Value) -> Result<PathBuf, OrbitError> {
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

    let Some(workspace_root) = ctx.workspace_root.as_deref() else {
        return Err(OrbitError::InvalidInput(
            "`knowledge_dir` is required when `workspace_root` is unavailable".to_string(),
        ));
    };
    Ok(workspace_root.join(".orbit/knowledge"))
}

/// Initialize a working graph by loading the persisted knowledge store,
/// or create a minimal graph by extracting the target file.
fn initialize_working_graph(
    knowledge_dir: &Path,
    selector: &Selector,
    workspace_root: &Path,
) -> Result<WorkingGraph, OrbitError> {
    use crate::knowledge::KnowledgeStore;
    use crate::knowledge::extractor::{self, Language};

    // Try to load from persisted store
    if let Ok(store) = KnowledgeStore::open(knowledge_dir)
        && let Ok(graph) = WorkingGraph::from_store(&store)
    {
        return Ok(graph);
    }

    // Fallback: extract the target file to build a minimal working graph
    let Selector::Leaf { path, .. } = selector else {
        return Ok(WorkingGraph::new());
    };

    let file_path = workspace_root.join(path);
    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let Some(language) = Language::from_extension(ext) else {
        return Ok(WorkingGraph::new());
    };

    let content = std::fs::read_to_string(&file_path)
        .map_err(|e| OrbitError::Execution(format!("read {}: {e}", file_path.display())))?;

    let extraction = extractor::extract_file(&content, language);
    let mut graph = WorkingGraph::new();

    // Populate from extraction
    for leaf in &extraction.leaves {
        let sel_str = format!("leaf:{path}#{}:{}", leaf.qualified_name, leaf.kind);
        let working_leaf = crate::knowledge::working_graph::WorkingLeaf {
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

    Ok(graph)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use crate::{Tool, ToolContext, ToolRegistry};

    use super::OrbitKnowledgeWriteTool;

    fn make_test_workspace() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let ws = dir.path().to_path_buf();

        // Create a Rust file to edit
        let src_dir = ws.join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(
            src_dir.join("lib.rs"),
            "use std::fmt;\n\npub fn hello() -> &'static str {\n    \"hello\"\n}\n\npub fn world() -> &'static str {\n    \"world\"\n}\n",
        ).unwrap();

        (dir, ws)
    }

    #[test]
    fn tool_is_registered_in_registry() {
        let mut registry = ToolRegistry::new();
        registry.register_builtins();
        assert!(registry.has("orbit.knowledge.write"));
    }

    #[test]
    fn edit_mode_replaces_leaf_source() {
        let (_dir, ws) = make_test_workspace();

        let result = OrbitKnowledgeWriteTool
            .execute(
                &ToolContext {
                    workspace_root: Some(ws.clone()),
                    ..Default::default()
                },
                json!({
                    "selector": "leaf:src/lib.rs#hello:function",
                    "new_source": "pub fn hello() -> &'static str {\n    \"hi there\"\n}",
                    "reason": "friendlier greeting"
                }),
            )
            .expect("edit should succeed");

        assert_eq!(result["status"], "ok");
        assert!(result["edit_sequence"].as_u64().unwrap() >= 1);
        assert!(result["new_source_hash"].as_str().is_some());

        // Verify file was actually modified
        let content = std::fs::read_to_string(ws.join("src/lib.rs")).unwrap();
        assert!(content.contains("hi there"));
        assert!(!content.contains("\"hello\""));
    }

    #[test]
    fn insert_mode_creates_new_leaf() {
        let (_dir, ws) = make_test_workspace();

        let result = OrbitKnowledgeWriteTool
            .execute(
                &ToolContext {
                    workspace_root: Some(ws.clone()),
                    ..Default::default()
                },
                json!({
                    "selector": "leaf:src/lib.rs#greet:function",
                    "new_source": "pub fn greet(name: &str) -> String {\n    format!(\"Hello, {name}!\")\n}",
                    "position": "after:leaf:src/lib.rs#hello:function",
                    "reason": "new greeting function"
                }),
            )
            .expect("insert should succeed");

        assert_eq!(result["status"], "created");
        assert_eq!(result["edit_sequence"], 0);

        let content = std::fs::read_to_string(ws.join("src/lib.rs")).unwrap();
        assert!(content.contains("pub fn greet(name: &str)"));
    }

    #[test]
    fn rejects_non_leaf_selector() {
        let err = OrbitKnowledgeWriteTool
            .execute(
                &ToolContext {
                    workspace_root: Some(PathBuf::from("/tmp")),
                    ..Default::default()
                },
                json!({
                    "selector": "file:src/lib.rs",
                    "new_source": "something"
                }),
            )
            .expect_err("should reject file selector");

        assert!(matches!(err, orbit_types::OrbitError::InvalidInput(_)));
    }

    #[test]
    fn rejects_missing_new_source() {
        let err = OrbitKnowledgeWriteTool
            .execute(
                &ToolContext {
                    workspace_root: Some(PathBuf::from("/tmp")),
                    ..Default::default()
                },
                json!({
                    "selector": "leaf:src/lib.rs#foo:function"
                }),
            )
            .expect_err("should reject missing new_source");

        assert!(matches!(err, orbit_types::OrbitError::InvalidInput(_)));
    }
}
