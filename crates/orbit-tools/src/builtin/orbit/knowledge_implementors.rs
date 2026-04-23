use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_knowledge::Selector;
use orbit_knowledge::service::GraphContextService;
use orbit_knowledge::service::implementors::trait_implementors;
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgeImplementorsTool;

impl Tool for OrbitKnowledgeImplementorsTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.implementors".to_string(),
            description: "Use when you need types implementing a trait. Prefer over grep when `impl Trait` misses qualified or re-exported names. Behavior: trailing identifiers match.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "trait_selector".to_string(),
                    description: "Trait selector.".to_string(),
                    param_type: "string".to_string(),
                    required: true,
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
        let selector_str = super::required_string(&input, &["trait_selector"], "trait_selector")?;
        let selector: Selector = selector_str
            .parse()
            .map_err(|e| OrbitError::InvalidInput(format!("{e}")))?;

        let graph = super::load_graph_for_read(ctx, &input)?;
        let svc = GraphContextService::new(&graph);

        let hits = trait_implementors(&svc, &graph, &selector)
            .map_err(|e| OrbitError::Execution(e.to_string()))?;

        let items: Vec<Value> = hits
            .into_iter()
            .map(|h| {
                json!({
                    "selector": h.selector,
                    "file": h.file,
                    "type_name": h.type_name,
                    "trait_path": h.trait_path,
                    "is_blanket": h.is_blanket,
                })
            })
            .collect();

        Ok(json!({
            "trait": selector_str,
            "total": items.len(),
            "implementors": items,
        }))
    }
}
