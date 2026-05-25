use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_knowledge::commands::implementors::{self, ImplementorsInput};
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
                super::super::graph_ref_param(),
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let result = implementors::run(ImplementorsInput {
            context: super::command_context(ctx, &input)?,
            trait_selector: super::super::required_string(
                &input,
                &["trait_selector"],
                "trait_selector",
            )?,
        })
        .map_err(super::knowledge_error_to_orbit)?;

        let items: Vec<Value> = result
            .implementors
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
            "trait": result.trait_selector,
            "total": items.len(),
            "implementors": items,
        }))
    }
}
