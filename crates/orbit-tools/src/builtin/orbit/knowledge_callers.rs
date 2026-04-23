use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_knowledge::Selector;
use orbit_knowledge::service::GraphContextService;
use orbit_knowledge::service::callers::{MAX_CALLER_DEPTH, transitive_callers};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

const DEFAULT_DEPTH: u64 = 2;

pub struct OrbitKnowledgeCallersTool;

impl Tool for OrbitKnowledgeCallersTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.callers".to_string(),
            description: "Use when you need callers of a symbol. Prefer over grep when text hits lose call paths. Behavior: BFS by simple name, capped by `depth`; use `orbit.graph.show` to confirm.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "selector".to_string(),
                    description: "Target symbol selector.".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "depth".to_string(),
                    description: "Max BFS depth.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "knowledge_dir".to_string(),
                    description: "Override knowledge dir.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                super::graph_ref_param(),
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let selector_str = super::required_string(&input, &["selector"], "selector")?;
        let selector: Selector = selector_str
            .parse()
            .map_err(|e| OrbitError::InvalidInput(format!("{e}")))?;

        let requested_depth = input
            .get("depth")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_DEPTH) as usize;
        let depth = requested_depth.min(MAX_CALLER_DEPTH);

        let graph = super::load_graph_for_read(ctx, &input)?;
        let svc = GraphContextService::new(&graph);

        let hits = transitive_callers(&svc, &graph, &selector, depth)
            .map_err(|e| OrbitError::Execution(e.to_string()))?;

        let items: Vec<Value> = hits
            .into_iter()
            .map(|h| {
                json!({
                    "selector": h.selector,
                    "name": h.name,
                    "file": h.file,
                    "kind": h.kind,
                    "distance": h.distance,
                    "via": h.via,
                })
            })
            .collect();

        Ok(json!({
            "target": selector_str,
            "depth": depth,
            "requested_depth": requested_depth,
            "total": items.len(),
            "callers": items,
        }))
    }
}
