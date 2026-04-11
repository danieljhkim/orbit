use std::path::Path;

use orbit_knowledge::graph::object_store::GraphObjectStore;
use orbit_knowledge::service::GraphContextService;
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgeSearchTool;

impl Tool for OrbitKnowledgeSearchTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.search".to_string(),
            description: "Search knowledge graph nodes by name or location".to_string(),
            parameters: vec![
                ToolParam {
                    name: "query".to_string(),
                    description: "Substring to match against node names and locations".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "type".to_string(),
                    description: "Filter by node type: dir, file, or symbol".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "prefix".to_string(),
                    description: "Filter by location prefix".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "limit".to_string(),
                    description: "Max results (default 20)".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let query = super::required_string(&input, &["query"], "query")?;
        let node_type = super::optional_string(&input, "type")?;
        let prefix = super::optional_string(&input, "prefix")?;
        let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;

        let workspace_root = ctx
            .workspace_root
            .as_deref()
            .ok_or_else(|| OrbitError::InvalidInput("workspace_root is required".to_string()))?;
        let graph_dir = Path::new(workspace_root).join(".orbit/knowledge/graph");
        let graph = GraphObjectStore::new(graph_dir)
            .read_graph()
            .map_err(|e| OrbitError::Execution(format!("failed to load knowledge graph: {e}")))?;

        let svc = GraphContextService::new(&graph);

        let type_strs: Vec<&str> = node_type.iter().map(String::as_str).collect();
        let node_types = if type_strs.is_empty() {
            None
        } else {
            Some(type_strs.as_slice())
        };

        let results = svc.search(&query, node_types, prefix.as_deref(), limit);

        let selectors: Vec<String> = results.iter().map(|n| svc.selector_for_node(*n)).collect();

        Ok(json!(selectors))
    }
}
