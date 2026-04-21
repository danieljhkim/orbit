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
            description: "Find transitive callers of a symbol (BFS upward, bounded by `depth`). Matching is by simple name: `self.foo()` and `other::foo()` both count as a call to any symbol named `foo`. Use for orientation; follow up with `orbit.graph.show` for precision.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "selector".to_string(),
                    description: "Symbol selector to find callers of (e.g. `symbol:path/to/file.rs#name:function`).".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "depth".to_string(),
                    description: "Maximum BFS depth (default 2, clamped to a small maximum).".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "knowledge_dir".to_string(),
                    description: "Optional knowledge artifact directory; defaults to `<workspace>/.orbit/knowledge`.".to_string(),
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
