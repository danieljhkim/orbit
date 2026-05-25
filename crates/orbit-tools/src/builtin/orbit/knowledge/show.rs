// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_knowledge::commands::show::{self, ShowInput, ShowNodeDetails, ShowResult};
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
                super::super::graph_ref_param(),
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let result = show::run(ShowInput {
            context: super::command_context(ctx, &input)?,
            selector: super::super::required_string(&input, &["selector"], "selector")?,
            depth: input.get("depth").and_then(Value::as_u64).unwrap_or(2) as usize,
            max_siblings: input.get("siblings").and_then(Value::as_u64).unwrap_or(3) as usize,
            max_children: input.get("children").and_then(Value::as_u64).unwrap_or(5) as usize,
        })
        .map_err(super::knowledge_error_to_orbit)?;

        Ok(show_response(result))
    }
}

fn show_response(result: ShowResult) -> Value {
    let mut value = json!({
        "selector": result.selector,
        "lineage": result.lineage,
        "siblings": result.siblings,
        "children": result.children,
    });
    let obj = value.as_object_mut().expect("show response object");

    match result.details {
        ShowNodeDetails::Leaf {
            source,
            start_line,
            end_line,
        } => {
            obj.insert("source".to_string(), json!(source));
            obj.insert("lines".to_string(), json!([start_line, end_line]));
        }
        ShowNodeDetails::File {
            source,
            source_blob_hash,
            imports,
            exports,
            re_exports,
        } => {
            if let Some(source) = source {
                obj.insert("source".to_string(), json!(source));
            }
            if let Some(source_blob_hash) = source_blob_hash {
                obj.insert("source_blob_hash".to_string(), json!(source_blob_hash));
            }
            if !imports.is_empty() {
                obj.insert("imports".to_string(), json!(imports));
            }
            if !exports.is_empty() {
                obj.insert("exports".to_string(), json!(exports));
            }
            if !re_exports.is_empty() {
                obj.insert("re_exports".to_string(), json!(re_exports));
            }
        }
        ShowNodeDetails::Dir => {}
    }

    value
}
