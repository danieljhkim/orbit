use orbit_common::types::{
    OrbitError, ToolParam, ToolSchema, optional_string, optional_string_list_alias,
};
use orbit_knowledge::{Selector, TaskGraphService};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgePackTool;

const DEFAULT_PACK_TIMEOUT_MS: u64 = 15_000;
const MAX_PACK_TIMEOUT_MS: u64 = 300_000;

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
                    name: "timeout_ms".to_string(),
                    description: "Maximum selector-packing time in milliseconds. Default 15000; returns partial unresolved selector entries on timeout.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "refresh".to_string(),
                    description: "Default false; use the existing graph snapshot instead of doing an inline auto-refresh. Set true only when a potentially slow refresh is acceptable.".to_string(),
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
        let selector_timeout_ms = parse_timeout_ms(&input)?;
        let refresh = parse_refresh(&input)?;
        let knowledge_dir = super::knowledge_write::resolve_knowledge_dir(ctx, &input)?;
        let explicit_ref = super::optional_string(&input, "ref")?;
        let explicit_knowledge_dir = super::has_explicit_knowledge_dir(&input);
        let skip_auto_refresh = !refresh || explicit_knowledge_dir;
        let service =
            TaskGraphService::new(knowledge_dir, super::knowledge_write::task_graph_scope(ctx));
        let mut pack = service.pack_json(
            &selectors,
            ctx.workspace_root.as_deref(),
            skip_auto_refresh,
            explicit_ref.as_deref(),
            Some(selector_timeout_ms),
        )?;
        add_refresh_diagnostics(
            &mut pack,
            refresh,
            explicit_ref.as_deref(),
            explicit_knowledge_dir,
        );

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

fn parse_timeout_ms(input: &Value) -> Result<u64, OrbitError> {
    let Some(value) = input.get("timeout_ms") else {
        return Ok(DEFAULT_PACK_TIMEOUT_MS);
    };
    if value.is_null() {
        return Ok(DEFAULT_PACK_TIMEOUT_MS);
    }
    let Some(timeout_ms) = value.as_u64() else {
        return Err(OrbitError::InvalidInput(
            "`timeout_ms` must be a non-negative integer".to_string(),
        ));
    };
    if timeout_ms > MAX_PACK_TIMEOUT_MS {
        return Err(OrbitError::InvalidInput(format!(
            "`timeout_ms` must be <= {MAX_PACK_TIMEOUT_MS}"
        )));
    }
    Ok(timeout_ms)
}

fn parse_refresh(input: &Value) -> Result<bool, OrbitError> {
    let Some(value) = input.get("refresh") else {
        return Ok(false);
    };
    if value.is_null() {
        return Ok(false);
    }
    value
        .as_bool()
        .ok_or_else(|| OrbitError::InvalidInput("`refresh` must be a boolean".to_string()))
}

fn add_refresh_diagnostics(
    pack: &mut Value,
    refresh: bool,
    explicit_ref: Option<&str>,
    explicit_knowledge_dir: bool,
) {
    if refresh || explicit_ref.is_some() || explicit_knowledge_dir {
        return;
    }
    let Some(obj) = pack.as_object_mut() else {
        return;
    };

    obj.insert(
        "diagnostics".to_string(),
        json!({
            "auto_refresh": {
                "status": "skipped",
                "reason": "orbit.graph.pack reads the existing graph snapshot by default so selector gathering returns promptly.",
                "remediation": "Run `orbit graph build` for an explicit refresh, or pass `refresh: true` when an inline refresh is acceptable."
            }
        }),
    );
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
