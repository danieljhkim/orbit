use orbit_knowledge::service::GraphContextService;
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgeSearchTool;

impl Tool for OrbitKnowledgeSearchTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.search".to_string(),
            description: "Search knowledge graph nodes by name or location. Omit query to browse all nodes.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "query".to_string(),
                    description: "Substring to match against node names and locations. Omit to list all nodes (browse mode).".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "type".to_string(),
                    description: "Filter by node type: dir, file, or symbol".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "kind".to_string(),
                    description: "Filter leaves by kind: function, method, struct, trait, impl, class, interface, field, module".to_string(),
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
                ToolParam {
                    name: "format".to_string(),
                    description: "Output format: 'structured' (default) returns objects with name/kind/file, 'selectors' returns flat selector strings".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let query = super::optional_string(&input, "query")?.unwrap_or_default();
        let node_type = super::optional_string(&input, "type")?;
        let kind_filter = super::optional_string(&input, "kind")?;
        let prefix = super::optional_string(&input, "prefix")?;
        let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
        let format = super::optional_string(&input, "format")?;
        let use_selectors = format.as_deref() == Some("selectors");

        let graph = super::load_graph_for_read(ctx, &input)?;
        let svc = GraphContextService::new(&graph);

        let type_strs: Vec<&str> = node_type.iter().map(String::as_str).collect();
        let node_types = if type_strs.is_empty() {
            None
        } else {
            Some(type_strs.as_slice())
        };

        let total = svc.search_total(
            &query,
            node_types,
            prefix.as_deref(),
            kind_filter.as_deref(),
        );

        if use_selectors {
            let results = svc.search(
                &query,
                node_types,
                prefix.as_deref(),
                kind_filter.as_deref(),
                limit,
            );
            let selectors: Vec<String> =
                results.iter().map(|n| svc.selector_for_node(*n)).collect();
            Ok(json!(selectors))
        } else {
            let results = svc.search_structured(
                &query,
                node_types,
                prefix.as_deref(),
                kind_filter.as_deref(),
                limit,
            );
            let items: Vec<Value> = results
                .into_iter()
                .map(|r| {
                    let mut obj = json!({
                        "selector": r.selector,
                        "name": r.name,
                        "kind": r.kind,
                    });
                    if let Some(file) = r.file {
                        obj["file"] = json!(file);
                    }
                    obj
                })
                .collect();
            Ok(json!({
                "total": total,
                "results": items,
            }))
        }
    }
}
