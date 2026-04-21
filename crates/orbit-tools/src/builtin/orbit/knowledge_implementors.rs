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
            description: "Find all `impl <Trait> for <Type>` blocks for a trait selector. Matches by trailing identifier, so `crate::foo::EngineHost` and `EngineHost` both match an `EngineHost` trait.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "trait_selector".to_string(),
                    description: "Symbol selector pointing at the trait (e.g. `symbol:crates/orbit-engine/src/context.rs#EngineHost:trait`).".to_string(),
                    param_type: "string".to_string(),
                    required: true,
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
