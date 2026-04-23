use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_knowledge::Selector;
use orbit_knowledge::graph::navigator::GraphNodeRef;
use orbit_knowledge::service::GraphContextService;
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgeShowTool;

impl Tool for OrbitKnowledgeShowTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.show".to_string(),
            description: "Use when you need one resolved node with nearby context. Prefer over grep when you need lineage, children, siblings, or source.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "selector".to_string(),
                    description: "Node selector.".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "depth".to_string(),
                    description: "Ancestor depth.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "siblings".to_string(),
                    description: "Max siblings.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "children".to_string(),
                    description: "Max children.".to_string(),
                    param_type: "number".to_string(),
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

        let depth = input.get("depth").and_then(Value::as_u64).unwrap_or(2) as usize;
        let max_siblings = input.get("siblings").and_then(Value::as_u64).unwrap_or(3) as usize;
        let max_children = input.get("children").and_then(Value::as_u64).unwrap_or(5) as usize;

        let graph = super::load_graph_for_read(ctx, &input)?;
        let svc = GraphContextService::new(&graph);

        let node = svc
            .resolve_selector(&selector)
            .map_err(|e| OrbitError::InvalidInput(e.to_string()))?;

        let node_ctx = svc
            .bounded_context(node.id(), depth, max_siblings, max_children)
            .map_err(|e| OrbitError::Execution(e.to_string()))?;

        Ok(build_json(&svc, &node_ctx))
    }
}

fn build_json(
    svc: &GraphContextService<'_>,
    ctx: &orbit_knowledge::service::NodeContext<'_>,
) -> Value {
    let node = ctx.node;
    let lineage: Vec<String> = ctx
        .lineage
        .iter()
        .map(|n| svc.selector_for_node(*n))
        .collect();
    let siblings: Vec<String> = ctx
        .siblings
        .iter()
        .map(|n| svc.selector_for_node(*n))
        .collect();
    let children: Vec<String> = ctx
        .children
        .iter()
        .map(|n| svc.selector_for_node(*n))
        .collect();

    let mut value = json!({
        "selector": svc.selector_for_node(node),
        "lineage": lineage,
        "siblings": siblings,
        "children": children,
    });

    match node {
        GraphNodeRef::Leaf(l) => {
            let obj = value.as_object_mut().unwrap();
            obj.insert("source".to_string(), json!(l.source));
            obj.insert("lines".to_string(), json!([l.start_line, l.end_line]));
        }
        GraphNodeRef::File(f) => {
            let obj = value.as_object_mut().unwrap();
            if !f.imports.is_empty() {
                obj.insert("imports".to_string(), json!(f.imports));
            }
            if !f.exports.is_empty() {
                obj.insert("exports".to_string(), json!(f.exports));
            }
        }
        GraphNodeRef::Dir(_) => {}
    }

    value
}
