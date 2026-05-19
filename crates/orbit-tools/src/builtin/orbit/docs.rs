use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitDocsListTool;
pub struct OrbitDocsShowTool;
pub struct OrbitDocsSearchTool;
pub struct OrbitDocsAddTool;
pub struct OrbitDocsReindexTool;
pub struct OrbitDocsMigrateTool;

impl Tool for OrbitDocsListTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.docs.list".to_string(),
            description: "List indexed Markdown docs under configured [docs].roots.".to_string(),
            parameters: vec![
                optional_param(
                    "type",
                    "Filter by doc type: design, pattern, context, glossary, or runbook.",
                    "string",
                ),
                optional_param("tag", "Filter by frontmatter tag.", "string"),
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::DocsList)
    }
}

impl Tool for OrbitDocsShowTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.docs.show".to_string(),
            description: "Show a single indexed doc with parsed frontmatter and Markdown body."
                .to_string(),
            parameters: vec![required_param(
                "path",
                "Repo-relative Markdown path to show.",
                "string",
            )],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::DocsShow)
    }
}

impl Tool for OrbitDocsSearchTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.docs.search".to_string(),
            description: "Search docs by frontmatter summary, tags, and type.".to_string(),
            parameters: vec![
                required_param("query", "Query text.", "string"),
                optional_param(
                    "limit",
                    "Maximum number of results. Default: 20.",
                    "integer",
                ),
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::DocsSearch)
    }
}

impl Tool for OrbitDocsAddTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.docs.add".to_string(),
            description:
                "Append an existing non-.orbit path to [docs].roots in .orbit/config.toml."
                    .to_string(),
            parameters: vec![required_param(
                "path",
                "Existing path to register as a docs root.",
                "string",
            )],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::DocsAdd)
    }
}

impl Tool for OrbitDocsReindexTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.docs.reindex".to_string(),
            description: "No-op v1 docs reindex surface; docs are walked on demand.".to_string(),
            parameters: Vec::new(),
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::DocsReindex)
    }
}

impl Tool for OrbitDocsMigrateTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.docs.migrate".to_string(),
            description: "Backfill locked docs frontmatter for legacy design and pattern docs."
                .to_string(),
            parameters: vec![optional_param(
                "dry_run",
                "Print planned diffs without writing.",
                "boolean",
            )],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::DocsMigrate)
    }
}

fn required_param(name: &str, description: &str, param_type: &str) -> ToolParam {
    ToolParam {
        name: name.to_string(),
        description: description.to_string(),
        param_type: param_type.to_string(),
        required: true,
    }
}

fn optional_param(name: &str, description: &str, param_type: &str) -> ToolParam {
    ToolParam {
        name: name.to_string(),
        description: description.to_string(),
        param_type: param_type.to_string(),
        required: false,
    }
}
