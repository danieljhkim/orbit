use std::collections::{BTreeMap, HashMap};

use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_knowledge::commands::overview::{
    self, DowngradeReason, GraphOverview, GraphOverviewSummary, OverviewBody, OverviewFormat,
    OverviewInput,
};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgeOverviewTool;

impl Tool for OrbitKnowledgeOverviewTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.overview".to_string(),
            description: "Use when you need repo shape. Prefer over grep when you want counts, not text hits. Behavior: broad scopes default to `summary`; use `format:\"full\"` for per-file symbols.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "prefix".to_string(),
                    description: "Only this path prefix.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "format".to_string(),
                    description: "`summary` or `full`.".to_string(),
                    param_type: "string".to_string(),
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
        let result = overview::run(OverviewInput {
            context: super::command_context(ctx, &input)?,
            prefix: super::super::optional_string(&input, "prefix")?,
            requested_format: parse_format(&input)?,
        })
        .map_err(super::knowledge_error_to_orbit)?;
        let requested_format = result.requested_format.label();

        Ok(match result.body {
            OverviewBody::Full(overview) => full_response(overview, requested_format),
            OverviewBody::Summary {
                summary,
                downgraded,
                downgrade_reason,
            } => summary_response(summary, requested_format, downgraded, downgrade_reason),
        })
    }
}

fn parse_format(input: &Value) -> Result<Option<OverviewFormat>, OrbitError> {
    let Some(format) = super::super::optional_string(input, "format")? else {
        return Ok(None);
    };

    match format.as_str() {
        "full" => Ok(Some(OverviewFormat::Full)),
        "summary" => Ok(Some(OverviewFormat::Summary)),
        other => Err(OrbitError::InvalidInput(format!(
            "`format` must be `full` or `summary`, got `{other}`"
        ))),
    }
}

fn full_response(overview: GraphOverview, requested_format: &str) -> Value {
    let files: Vec<Value> = overview
        .files
        .into_iter()
        .map(|f| {
            let symbols: Vec<Value> = f
                .symbols
                .into_iter()
                .map(|s| {
                    json!({
                        "name": s.name,
                        "kind": s.kind,
                        "selector": s.selector,
                    })
                })
                .collect();
            json!({
                "selector": f.selector,
                "name": f.name,
                "symbol_count": f.symbol_count,
                "symbols": symbols,
            })
        })
        .collect();

    json!({
        "mode": "full",
        "requested_format": requested_format,
        "total_dirs": overview.total_dirs,
        "total_files": overview.total_files,
        "total_symbols": overview.total_symbols,
        "languages": overview.languages,
        "symbol_kinds": overview.symbol_kinds,
        "files": files,
    })
}

fn summary_response(
    summary: GraphOverviewSummary,
    requested_format: &str,
    downgraded: bool,
    downgrade_reason: Option<DowngradeReason>,
) -> Value {
    let top_files: Vec<Value> = summary
        .top_files
        .into_iter()
        .map(|file| {
            json!({
                "selector": file.selector,
                "name": file.name,
                "symbol_count": file.symbol_count,
            })
        })
        .collect();

    let mut response = json!({
        "mode": "summary",
        "requested_format": requested_format,
        "downgraded": downgraded,
        "total_dirs": summary.total_dirs,
        "total_files": summary.total_files,
        "total_symbols": summary.total_symbols,
        "languages": sorted_counts(summary.languages),
        "symbol_kinds": sorted_counts(summary.symbol_kinds),
        "dir_file_counts": summary.dir_file_counts,
        "top_files": top_files,
        "hint": summary.hint,
    });
    if let Some(reason) = downgrade_reason
        && let Some(object) = response.as_object_mut()
    {
        object.insert(
            "downgrade_reason".to_string(),
            downgrade_reason_value(reason),
        );
    }
    response
}

fn sorted_counts(counts: HashMap<String, usize>) -> BTreeMap<String, usize> {
    counts.into_iter().collect()
}

fn downgrade_reason_value(reason: DowngradeReason) -> Value {
    match reason {
        DowngradeReason::FileThreshold { threshold, actual } => json!({
            "kind": "file_threshold",
            "threshold": threshold,
            "actual": actual,
        }),
    }
}
