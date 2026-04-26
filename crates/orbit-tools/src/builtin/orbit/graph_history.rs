//! `orbit.graph.history` agent tool (T20260426-0507).
//!
//! Surfaces task-ID history for a knowledge-graph selector, mirroring the
//! `orbit graph history` CLI. Returns the same JSON shape as the CLI's
//! `--json` output so agents and humans share one schema.

use std::path::Path;

use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_knowledge::graph::object_store::resolve_graph_read_target;
use orbit_knowledge::{
    DEFAULT_STALENESS_THRESHOLD, HistoryQueryOptions, Selector, TaskIdPattern, query_task_history,
};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct OrbitGraphHistoryTool;

impl Tool for OrbitGraphHistoryTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.history".to_string(),
            description: "Use when you need which task IDs touched a file/symbol/dir. \
                 Prefer over grep when commit-message scanning would miss the \
                 task→node attribution maintained by the graph. Behavior: \
                 graph-backed lookup of `task_ids` on the node + sidecar; falls \
                 back to a `git log` regex scan when the graph is missing. \
                 Capture-group convention: if `task_id_pattern` has a capture \
                 group, group 1 is the ID; otherwise the whole match. Default \
                 pattern strips Orbit `[T...]` brackets via group 1."
                .to_string(),
            parameters: vec![
                ToolParam {
                    name: "selector".to_string(),
                    description:
                        "Knowledge-graph selector (file:path, symbol:path#name:kind, dir:path)."
                            .to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "task_id_pattern".to_string(),
                    description: "Override the task-ID extraction regex. Falls back to workspace \
                         config and then the Orbit default."
                        .to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "staleness_threshold".to_string(),
                    description: "Commits-behind-HEAD threshold for the staleness warning."
                        .to_string(),
                    param_type: "number".to_string(),
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
        let selector_str = super::required_string(&input, &["selector"], "selector")?;
        let selector: Selector = selector_str
            .parse()
            .map_err(|error| OrbitError::InvalidInput(format!("{error}")))?;

        let task_id_pattern_input = super::optional_string(&input, "task_id_pattern")?;
        let workspace_pattern = ctx
            .orbit_host
            .as_ref()
            .and_then(|host| host.task_id_pattern());

        let task_id_pattern = if let Some(raw) = task_id_pattern_input.as_deref() {
            TaskIdPattern::new(raw).map_err(|error| OrbitError::InvalidInput(error.reason))?
        } else if let Some(raw) = workspace_pattern.as_deref() {
            TaskIdPattern::new(raw).map_err(|error| OrbitError::InvalidInput(error.reason))?
        } else {
            TaskIdPattern::default()
        };

        let staleness_threshold = input
            .get("staleness_threshold")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_STALENESS_THRESHOLD);

        let explicit_ref = super::optional_string(&input, "ref")?;

        let knowledge_dir =
            super::knowledge_write::resolve_knowledge_dir(ctx, &input).map_err(|error| {
                OrbitError::InvalidInput(format!(
                    "could not resolve knowledge_dir for orbit.graph.history: {error}"
                ))
            })?;
        let workspace_root = ctx
            .workspace_root
            .as_deref()
            .unwrap_or_else(|| Path::new("."));
        let read_target = resolve_graph_read_target(Some(workspace_root), explicit_ref.as_deref())
            .map_err(|error| OrbitError::InvalidInput(format!("{error}")))?;
        let branch_ref = read_target.requested.clone();

        let options = HistoryQueryOptions {
            knowledge_dir: &knowledge_dir,
            repo_path: workspace_root,
            branch_ref: &branch_ref,
            selector: &selector,
            staleness_threshold,
            task_id_pattern: &task_id_pattern,
        };

        let result = query_task_history(&options).map_err(|error| {
            OrbitError::Execution(format!("orbit.graph.history failed: {error}"))
        })?;

        Ok(json!({
            "selector": result.selector,
            "source": result.source,
            "task_history": result.task_history,
            "staleness": result.staleness,
            "structural_conflict": result.structural_conflict,
            "warnings": result.warnings,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_lists_required_selector_and_optional_pattern() {
        let tool = OrbitGraphHistoryTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "orbit.graph.history");
        let selector = schema
            .parameters
            .iter()
            .find(|p| p.name == "selector")
            .expect("selector param present");
        assert!(selector.required);
        let pattern = schema
            .parameters
            .iter()
            .find(|p| p.name == "task_id_pattern")
            .expect("task_id_pattern param present");
        assert!(!pattern.required);
        let staleness = schema
            .parameters
            .iter()
            .find(|p| p.name == "staleness_threshold")
            .expect("staleness_threshold param present");
        assert!(!staleness.required);
        assert_eq!(staleness.param_type, "number");
    }

    #[test]
    fn schema_description_documents_capture_group_convention() {
        let schema = OrbitGraphHistoryTool.schema();
        assert!(
            schema.description.contains("Capture-group convention"),
            "description should mention capture-group convention: {}",
            schema.description
        );
    }
}
