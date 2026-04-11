use std::path::{Path, PathBuf};

use orbit_knowledge::{
    KnowledgeStore, Selector, load_task_working_graph, overlay_pack_with_working_graph,
    pack_from_working_graph,
};
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgePackTool;

impl Tool for OrbitKnowledgePackTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.pack".to_string(),
            description:
                "Resolve knowledge selectors into a scoped pack from `.orbit/knowledge` artifacts"
                    .to_string(),
            parameters: vec![
                ToolParam {
                    name: "selectors".to_string(),
                    description: "Selector strings like `file:path`, `symbol:path#symbol:kind`, or `dir:path`".to_string(),
                    param_type: "array".to_string(),
                    required: true,
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
        let selectors = parse_selector_strings(&input)?;
        let selectors = Selector::parse_many(&selectors)
            .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;
        let knowledge_dir = resolve_knowledge_dir(ctx, &input)?;
        let working_graph =
            load_task_working_graph(ctx.orbit_root.as_deref(), ctx.task_id.as_deref())?;

        let store = match KnowledgeStore::open(&knowledge_dir) {
            Ok(store) => store,
            Err(error) => {
                if let Some(graph) = working_graph.as_ref() {
                    let pack = pack_from_working_graph(&knowledge_dir, &selectors, graph);
                    return serde_json::to_value(pack).map_err(|serialize| {
                        OrbitError::Execution(format!(
                            "failed to serialize knowledge pack: {serialize}"
                        ))
                    });
                }
                return serde_json::to_value(error).map_err(|serialize| {
                    OrbitError::Execution(format!(
                        "failed to serialize knowledge error: {serialize}"
                    ))
                });
            }
        };
        let pack = match store.pack(&selectors) {
            Ok(pack) => pack,
            Err(error) => {
                return serde_json::to_value(error).map_err(|serialize| {
                    OrbitError::Execution(format!(
                        "failed to serialize knowledge error: {serialize}"
                    ))
                });
            }
        };
        let pack = if let Some(graph) = working_graph.as_ref() {
            overlay_pack_with_working_graph(pack, &selectors, graph)
        } else {
            pack
        };

        serde_json::to_value(pack)
            .map_err(|error| OrbitError::Execution(format!("serialize knowledge pack: {error}")))
    }
}

fn parse_selector_strings(input: &Value) -> Result<Vec<String>, OrbitError> {
    let raw = input
        .get("selectors")
        .ok_or_else(|| OrbitError::InvalidInput("missing `selectors`".to_string()))?;
    let items = raw
        .as_array()
        .ok_or_else(|| OrbitError::InvalidInput("`selectors` must be an array".to_string()))?;
    if items.is_empty() {
        return Err(OrbitError::InvalidInput(
            "`selectors` must contain at least one selector".to_string(),
        ));
    }

    items
        .iter()
        .map(|item| {
            item.as_str().map(str::to_string).ok_or_else(|| {
                OrbitError::InvalidInput("`selectors` entries must be strings".to_string())
            })
        })
        .collect()
}

fn resolve_knowledge_dir(ctx: &ToolContext, input: &Value) -> Result<PathBuf, OrbitError> {
    if let Some(raw) = input.get("knowledge_dir") {
        let raw = raw.as_str().ok_or_else(|| {
            OrbitError::InvalidInput("`knowledge_dir` must be a string".to_string())
        })?;
        if raw.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "`knowledge_dir` must not be empty".to_string(),
            ));
        }
        return Ok(resolve_path(ctx, Path::new(raw)));
    }

    let Some(workspace_root) = ctx.workspace_root.as_deref() else {
        return Err(OrbitError::InvalidInput(
            "`knowledge_dir` is required when `workspace_root` is unavailable".to_string(),
        ));
    };
    Ok(workspace_root.join(".orbit/knowledge"))
}

fn resolve_path(ctx: &ToolContext, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    if let Some(workspace_root) = ctx.workspace_root.as_deref() {
        return workspace_root.join(path);
    }
    if let Some(cwd) = ctx.cwd.as_deref() {
        return Path::new(cwd).join(path);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::{Value, json};

    use crate::builtin::orbit::knowledge_write::OrbitKnowledgeWriteTool;
    use crate::{Tool, ToolContext, ToolRegistry};

    use super::OrbitKnowledgePackTool;

    fn fixture_knowledge_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/knowledge")
            .join(".orbit/knowledge")
    }

    fn make_test_workspace() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let ws = dir.path().to_path_buf();

        let src_dir = ws.join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::create_dir_all(ws.join(".orbit")).unwrap();
        std::fs::write(
            src_dir.join("lib.rs"),
            "pub fn hello() -> &'static str {\n    \"hello\"\n}\n",
        )
        .unwrap();

        (dir, ws)
    }

    fn task_context(ws: &std::path::Path, task_id: &str) -> ToolContext {
        ToolContext {
            workspace_root: Some(ws.to_path_buf()),
            orbit_root: Some(ws.join(".orbit")),
            task_id: Some(task_id.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn registry_executes_knowledge_pack_tool() {
        let mut registry = ToolRegistry::new();
        registry.register_builtins();

        let result = registry
            .execute(
                "orbit.graph.pack",
                &ToolContext {
                    workspace_root: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR"))),
                    ..Default::default()
                },
                json!({
                    "selectors": [
                        "file:crates/orbit-tools/src/lib.rs",
                        "symbol:crates/orbit-tools/src/lib.rs#register_builtins:function"
                    ],
                    "knowledge_dir": fixture_knowledge_dir().display().to_string()
                }),
            )
            .expect("tool should succeed");

        assert_eq!(
            result["knowledge_dir"],
            fixture_knowledge_dir().display().to_string()
        );
        assert_eq!(result["manifest_generated_at"], "2026-04-09T06:06:39Z");
        assert_eq!(result["total_nodes"], 2);
        let entries = result["entries"].as_array().expect("entries array");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["selector"], "file:crates/orbit-tools/src/lib.rs");
        assert_eq!(
            entries[1]["selector"],
            "symbol:crates/orbit-tools/src/lib.rs#register_builtins:function"
        );
    }

    #[test]
    fn tool_returns_structured_error_for_missing_knowledge() {
        let result = OrbitKnowledgePackTool
            .execute(
                &ToolContext {
                    workspace_root: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR"))),
                    ..Default::default()
                },
                json!({
                    "selectors": ["file:crates/orbit-tools/src/lib.rs"],
                    "knowledge_dir": "/tmp/orbit-tools-missing-knowledge"
                }),
            )
            .expect("structured error should be returned as JSON");

        assert_eq!(result["kind"], "knowledge_unavailable");
        assert!(
            result["reason"]
                .as_str()
                .is_some_and(|reason| !reason.is_empty())
        );
    }

    #[test]
    fn tool_rejects_non_array_selector_input() {
        let error = OrbitKnowledgePackTool
            .execute(
                &ToolContext::default(),
                json!({
                    "selectors": "file:crates/orbit-tools/src/lib.rs"
                }),
            )
            .expect_err("invalid selector input should fail");

        assert!(matches!(error, orbit_types::OrbitError::InvalidInput(_)));
    }

    #[test]
    fn tool_serializes_success_to_json() {
        let value = OrbitKnowledgePackTool
            .execute(
                &ToolContext {
                    workspace_root: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR"))),
                    ..Default::default()
                },
                json!({
                    "selectors": ["dir:crates/orbit-tools/src"],
                    "knowledge_dir": fixture_knowledge_dir().display().to_string()
                }),
            )
            .expect("pack");

        assert_eq!(
            value["entries"][0]["kind"],
            Value::String("dir".to_string())
        );
    }

    #[test]
    fn pack_uses_task_scoped_working_graph_after_write() {
        let (_dir, ws) = make_test_workspace();
        let ctx = task_context(&ws, "T-pack");

        OrbitKnowledgeWriteTool
            .execute(
                &ctx,
                json!({
                    "selector": "symbol:src/lib.rs#hello:function",
                    "new_source": "pub fn hello() -> &'static str {\n    \"hi there\"\n}",
                    "reason": "updated greeting"
                }),
            )
            .expect("write");

        let result = OrbitKnowledgePackTool
            .execute(
                &ctx,
                json!({
                    "selectors": ["symbol:src/lib.rs#hello:function"]
                }),
            )
            .expect("pack");

        assert_eq!(result["total_nodes"], 1);
        assert_eq!(
            result["entries"][0]["source"],
            Value::String("pub fn hello() -> &'static str {\n    \"hi there\"\n}".to_string())
        );
        assert!(
            result["unresolved_selectors"]
                .as_array()
                .expect("unresolved selectors")
                .is_empty()
        );
    }
}
