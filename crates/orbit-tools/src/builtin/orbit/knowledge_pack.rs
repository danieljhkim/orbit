use orbit_common::types::{
    OrbitError, ToolParam, ToolSchema, optional_string, optional_string_list_alias,
};
use orbit_knowledge::{Selector, TaskGraphService};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgePackTool;

impl Tool for OrbitKnowledgePackTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.pack".to_string(),
            description:
                "Use when you need exact selectors with context. Prefer over grep when raw text pulls the wrong symbols. Behavior: `file:` stays metadata-only; `summary` hides leaf bodies unless false."
                    .to_string(),
            parameters: vec![
                ToolParam {
                    name: "selectors".to_string(),
                    description: "Selector string or array.".to_string(),
                    param_type: "string_list".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "summary".to_string(),
                    description: "Default true; drop leaf bodies.".to_string(),
                    param_type: "boolean".to_string(),
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
        let selectors = parse_selector_strings(&input)?;
        let selectors = Selector::parse_many(&selectors)
            .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;
        let summary = input
            .get("summary")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let knowledge_dir = super::knowledge_write::resolve_knowledge_dir(ctx, &input)?;
        let explicit_ref = super::optional_string(&input, "ref")?;
        let service =
            TaskGraphService::new(knowledge_dir, super::knowledge_write::task_graph_scope(ctx));
        let pack = service.pack_json(
            &selectors,
            ctx.workspace_root.as_deref(),
            super::has_explicit_knowledge_dir(&input),
            explicit_ref.as_deref(),
        )?;

        Ok(if summary {
            summarize_pack_json(pack)
        } else {
            pack
        })
    }
}

fn parse_selector_strings(input: &Value) -> Result<Vec<String>, OrbitError> {
    let selectors = if let Some(selectors) = optional_string_list_alias(input, &["selectors"])? {
        selectors
    } else if let Some(file) = optional_string(input, "file")? {
        let selector = if file.starts_with("file:") {
            file
        } else {
            format!("file:{file}")
        };
        vec![selector]
    } else {
        return Err(OrbitError::InvalidInput("missing `selectors`".to_string()));
    };
    if selectors.is_empty() {
        return Err(OrbitError::InvalidInput(
            "`selectors` must contain at least one selector".to_string(),
        ));
    }
    Ok(selectors)
}

fn summarize_pack_json(mut pack: Value) -> Value {
    let Some(entries) = pack.get_mut("entries").and_then(Value::as_array_mut) else {
        return pack;
    };

    for entry in entries {
        summarize_pack_entry(entry);
    }

    pack
}

fn summarize_pack_entry(entry: &mut Value) {
    let Some(obj) = entry.as_object_mut() else {
        return;
    };
    if obj.get("kind").and_then(Value::as_str) != Some("leaf") {
        return;
    }

    obj.remove("source");

    let Some(selector) = obj.get("selector").and_then(Value::as_str) else {
        return;
    };
    let Some(file_path) = selector
        .strip_prefix("symbol:")
        .and_then(|rest| rest.split_once('#').map(|(path, _)| path.to_string()))
    else {
        return;
    };
    obj.insert("file".to_string(), Value::String(file_path));
}
