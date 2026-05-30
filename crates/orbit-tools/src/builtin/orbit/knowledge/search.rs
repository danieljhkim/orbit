use orbit_common::types::{OrbitError, ToolParam, ToolSchema, optional_raw_string};
use orbit_knowledge::commands::search::{self, SOURCE_REGEX_UNBOUNDED_LIMIT_MAX, SearchInput};
use regex::Regex;
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgeSearchTool;

const DEFAULT_LIMIT: usize = 20;

impl Tool for OrbitKnowledgeSearchTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.search".to_string(),
            description: "Use when you need to locate selectors by name/path/source regex. Prefer over grep when text hits lose node type or source-context filters. Default ranking keeps at most 5000 candidates before ranking.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "query".to_string(),
                    description: "Name/path.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "type".to_string(),
                    description: "dir/file/symbol.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "kind".to_string(),
                    description: "Kind.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "prefix".to_string(),
                    description: "Prefix.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "source_regex".to_string(),
                    description:
                        "Rust regex; empty query needs prefix/limit<=200; adds matched_lines."
                            .to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "limit".to_string(),
                    description: "Limit.".to_string(),
                    param_type: "number".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "include_non_code".to_string(),
                    description: "Docs/configs.".to_string(),
                    param_type: "boolean".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "allow_fuzzy".to_string(),
                    description:
                        "Enable fuzzy name fallback when the deterministic pass returns zero results."
                            .to_string(),
                    param_type: "boolean".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "format".to_string(),
                    description: "structured/selectors.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                super::super::graph_ref_param(),
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let query = optional_raw_string(&input, "query")?
            .map(|value| value.trim().to_string())
            .unwrap_or_default();
        let node_type = super::super::optional_string(&input, "type")?;
        let kind_filter = super::super::optional_string(&input, "kind")?;
        let prefix = super::super::optional_string(&input, "prefix")?;
        let source_regex_text = super::super::optional_string(&input, "source_regex")?;
        let limit_provided = input.get("limit").is_some();
        let limit = input
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_LIMIT as u64) as usize;
        validate_source_regex_bound(
            &query,
            prefix.as_deref(),
            limit,
            limit_provided,
            &source_regex_text,
        )?;
        let source_regex = compile_source_regex(source_regex_text.as_deref())?;
        let has_source_regex = source_regex.is_some();
        let include_non_code = input
            .get("include_non_code")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let allow_fuzzy = input
            .get("allow_fuzzy")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let format = super::super::optional_string(&input, "format")?;
        let use_selectors = format.as_deref() == Some("selectors");

        let result = search::run(SearchInput {
            context: super::command_context(ctx, &input)?,
            query,
            node_type,
            kind_filter,
            prefix,
            source_regex,
            include_non_code,
            allow_fuzzy,
            limit,
        })
        .map_err(super::knowledge_error_to_orbit)?;

        if use_selectors {
            Ok(json!(
                result
                    .hits
                    .into_iter()
                    .map(|hit| hit.selector)
                    .collect::<Vec<_>>()
            ))
        } else {
            Ok(json!({
                "total": result.total,
                "results": result.hits.into_iter().map(|hit| {
                    let mut obj = json!({
                        "selector": hit.selector,
                        "name": hit.name,
                        "kind": hit.kind,
                    });
                    if let Some(file) = hit.file {
                        obj["file"] = json!(file);
                    }
                    if let Some(match_kind) = hit.match_kind {
                        obj["match_kind"] = json!(match_kind);
                    }
                    if let Some(score) = hit.score {
                        obj["score"] = json!(score);
                    }
                    if has_source_regex {
                        obj["matched_lines"] = json!(
                            hit.matched_lines.into_iter().map(|line| json!({
                                "line_number": line.line_number,
                                "snippet": line.snippet,
                            })).collect::<Vec<_>>()
                        );
                    }
                    obj
                }).collect::<Vec<_>>(),
            }))
        }
    }
}

fn validate_source_regex_bound(
    query: &str,
    prefix: Option<&str>,
    limit: usize,
    limit_provided: bool,
    source_regex: &Option<String>,
) -> Result<(), OrbitError> {
    if source_regex.is_none() || prefix.is_some() || !query.trim().is_empty() {
        return Ok(());
    }
    if limit_provided && limit <= SOURCE_REGEX_UNBOUNDED_LIMIT_MAX {
        return Ok(());
    }

    Err(OrbitError::InvalidInput(format!(
        "`source_regex` without `prefix` or non-empty `query` requires explicit `limit` <= {SOURCE_REGEX_UNBOUNDED_LIMIT_MAX}"
    )))
}

fn compile_source_regex(source_regex: Option<&str>) -> Result<Option<Regex>, OrbitError> {
    source_regex
        .map(|pattern| {
            Regex::new(pattern).map_err(|error| {
                OrbitError::InvalidInput(format!("invalid `source_regex`: {error}"))
            })
        })
        .transpose()
}
