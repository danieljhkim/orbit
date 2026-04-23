use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_knowledge::service::{
    GraphContextService, GraphOverview, GraphOverviewSummary, compact_from_overview,
};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgeOverviewTool;

const AUTO_SUMMARY_FILE_THRESHOLD: usize = 20;
const FILE_THRESHOLD: usize = 50;
const SUMMARY_HINT: &str = "Use `prefix` to narrow the overview and get per-file symbol listings.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverviewFormat {
    Full,
    Summary,
}

impl OverviewFormat {
    fn parse(input: &Value) -> Result<Option<Self>, OrbitError> {
        let Some(format) = super::optional_string(input, "format")? else {
            return Ok(None);
        };

        match format.as_str() {
            "full" => Ok(Some(Self::Full)),
            "summary" => Ok(Some(Self::Summary)),
            other => Err(OrbitError::InvalidInput(format!(
                "`format` must be `full` or `summary`, got `{other}`"
            ))),
        }
    }

    fn default_for_scope(prefix: Option<&str>, file_count: usize) -> Self {
        if prefix.is_none() || file_count > AUTO_SUMMARY_FILE_THRESHOLD {
            Self::Summary
        } else {
            Self::Full
        }
    }
}

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
                super::graph_ref_param(),
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let prefix = super::optional_string(&input, "prefix")?;
        let requested_format = OverviewFormat::parse(&input)?;
        let graph = super::load_graph_for_read(ctx, &input)?;
        let svc = GraphContextService::new(&graph);
        let overview = svc.overview(prefix.as_deref());
        let resolved_format = requested_format.unwrap_or_else(|| {
            OverviewFormat::default_for_scope(prefix.as_deref(), overview.files.len())
        });
        let downgraded = matches!(requested_format, Some(OverviewFormat::Full))
            && overview.files.len() > FILE_THRESHOLD;
        let use_summary = matches!(resolved_format, OverviewFormat::Summary) || downgraded;
        let requested_format = match requested_format {
            Some(OverviewFormat::Full) => "full",
            Some(OverviewFormat::Summary) => "summary",
            None => "auto",
        };

        Ok(if use_summary {
            summary_response(
                compact_from_overview(&overview, prefix.as_deref(), SUMMARY_HINT),
                requested_format,
                downgraded,
            )
        } else {
            full_response(overview, requested_format)
        })
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

    json!({
        "mode": "summary",
        "requested_format": requested_format,
        "downgraded": downgraded,
        "total_dirs": summary.total_dirs,
        "total_files": summary.total_files,
        "total_symbols": summary.total_symbols,
        "languages": summary.languages,
        "symbol_kinds": summary.symbol_kinds,
        "dir_file_counts": summary.dir_file_counts,
        "top_files": top_files,
        "hint": summary.hint,
    })
}
