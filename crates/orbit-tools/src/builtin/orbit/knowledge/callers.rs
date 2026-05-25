use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_knowledge::commands::callers::{self, CallersInput};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

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
                super::super::graph_ref_param(),
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let result = callers::run(CallersInput {
            context: super::command_context(ctx, &input)?,
            selector: super::super::required_string(&input, &["selector"], "selector")?,
            requested_depth: input
                .get("depth")
                .and_then(Value::as_u64)
                .map(|value| value as usize),
        })
        .map_err(super::knowledge_error_to_orbit)?;

        let items: Vec<Value> = result
            .callers
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
            "target": result.target,
            "depth": result.depth,
            "requested_depth": result.requested_depth,
            "total": items.len(),
            "callers": items,
        }))
    }
}
