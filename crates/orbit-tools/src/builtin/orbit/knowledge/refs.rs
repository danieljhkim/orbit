use orbit_common::types::{OrbitError, ToolParam, ToolSchema, optional_string_list_alias};
use orbit_knowledge::commands::refs::{self, RefInclude, RefMatch, RefsInput};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgeRefsTool;

const DEFAULT_LIMIT: usize = 20;
const DEFAULT_PER_FILE_LIMIT: usize = 5;

impl Tool for OrbitKnowledgeRefsTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.refs".to_string(),
            description: "Use when you need symbol refs. Prefer over grep when raw hits mix code, docs, and config. Behavior: returns `code_refs`; `doc_refs` and `config_refs` stay empty unless `include` asks for them.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "selector".to_string(),
                    description: "Target symbol selector.".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "limit".to_string(),
                    description: "Max results.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "include_simple_name".to_string(),
                    description: "Also search the tail name.".to_string(),
                    param_type: "boolean".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "include".to_string(),
                    description: "String/array: `code`, `doc`, `config`, `all`.".to_string(),
                    param_type: "string_list".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "per_file_limit".to_string(),
                    description: "Max refs per file/category.".to_string(),
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
        let result = refs::run(RefsInput {
            context: super::command_context(ctx, &input)?,
            selector: super::super::required_string(&input, &["selector"], "selector")?,
            include_simple_name: input
                .get("include_simple_name")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            include: parse_include(&input)?,
            limit: input
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(DEFAULT_LIMIT as u64) as usize,
            per_file_limit: input
                .get("per_file_limit")
                .and_then(Value::as_u64)
                .unwrap_or(DEFAULT_PER_FILE_LIMIT as u64) as usize,
        })
        .map_err(super::knowledge_error_to_orbit)?;

        Ok(json!({
            "code_refs": refs_to_json(result.code_refs),
            "doc_refs": refs_to_json(result.doc_refs),
            "config_refs": refs_to_json(result.config_refs),
        }))
    }
}

fn parse_include(input: &Value) -> Result<RefInclude, OrbitError> {
    if input.get("include").is_none() {
        return Ok(RefInclude::code_only());
    }
    RefInclude::from_names(optional_string_list_alias(input, &["include"])?.unwrap_or_default())
        .map_err(super::knowledge_error_to_orbit)
}

fn refs_to_json(refs: Vec<RefMatch>) -> Vec<Value> {
    refs.into_iter()
        .map(|hit| {
            json!({
                "selector": hit.selector,
                "name": hit.name,
                "file": hit.file,
                "kind": hit.kind,
            })
        })
        .collect()
}
