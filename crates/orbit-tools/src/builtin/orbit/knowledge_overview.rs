use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_knowledge::service::{
    GraphContextService, GraphOverview, GraphOverviewSummary, compact_from_overview,
};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgeOverviewTool;

const FILE_THRESHOLD: usize = 50;
const SUMMARY_HINT: &str = "Use `prefix` to narrow the overview and get per-file symbol listings.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverviewFormat {
    Full,
    Summary,
}

impl OverviewFormat {
    fn parse(input: &Value) -> Result<Self, OrbitError> {
        let Some(format) = super::optional_string(input, "format")? else {
            return Ok(Self::Full);
        };

        match format.as_str() {
            "full" => Ok(Self::Full),
            "summary" => Ok(Self::Summary),
            other => Err(OrbitError::InvalidInput(format!(
                "`format` must be `full` or `summary`, got `{other}`"
            ))),
        }
    }
}

impl Tool for OrbitKnowledgeOverviewTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.overview".to_string(),
            description: "Return an aggregate overview of the knowledge graph. Broad queries auto-compact above 50 files, and `format: \"summary\"` always forces the compact form.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "prefix".to_string(),
                    description: "Location prefix to scope the overview (e.g. `crates/orbit-knowledge/src`)".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "format".to_string(),
                    description: "Output format: `full` (default) returns per-file symbol listings when the scope stays under 50 files, but broad queries may still auto-compact to summary above 50 files; `summary` always returns the compact form.".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "knowledge_dir".to_string(),
                    description: "Optional knowledge artifact directory; defaults to `<workspace>/.orbit/knowledge`".to_string(),
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
        let format = OverviewFormat::parse(&input)?;
        let graph = super::load_graph_for_read(ctx, &input)?;
        let svc = GraphContextService::new(&graph);
        let overview = svc.overview(prefix.as_deref());
        let downgraded =
            matches!(format, OverviewFormat::Full) && overview.files.len() > FILE_THRESHOLD;
        let use_summary = matches!(format, OverviewFormat::Summary) || downgraded;
        let requested_format = match format {
            OverviewFormat::Full => "full",
            OverviewFormat::Summary => "summary",
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
