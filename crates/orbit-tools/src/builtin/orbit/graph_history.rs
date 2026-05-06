//! `orbit.graph.history` compatibility tool.

use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_knowledge::Selector;
use serde_json::Value;

use crate::graph::REMOVED_GRAPH_HISTORY_MESSAGE;
use crate::{Tool, ToolContext};

pub struct OrbitGraphHistoryTool;

impl Tool for OrbitGraphHistoryTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.history".to_string(),
            description: "Use when legacy clients need a structured deprecation error for removed knowledge-graph task attribution. Use instead of grep when the caller must receive Orbit's compatibility response; otherwise run `git log --grep '[T<task-id>]'` locally."
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

    fn execute(&self, _ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let selector_str = super::required_string(&input, &["selector"], "selector")?;
        let selector: Selector = selector_str
            .parse()
            .map_err(|error| OrbitError::InvalidInput(format!("{error}")))?;
        let _ = selector;

        Err(OrbitError::InvalidInput(
            REMOVED_GRAPH_HISTORY_MESSAGE.to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_lists_required_selector_without_task_id_pattern() {
        let tool = OrbitGraphHistoryTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "orbit.graph.history");
        let selector = schema
            .parameters
            .iter()
            .find(|p| p.name == "selector")
            .expect("selector param present");
        assert!(selector.required);
        assert!(
            !schema
                .parameters
                .iter()
                .any(|p| p.name == "task_id_pattern")
        );
    }

    #[test]
    fn schema_description_points_to_git_grep() {
        let schema = OrbitGraphHistoryTool.schema();
        assert!(
            schema.description.contains("git log --grep"),
            "description should mention git grep replacement: {}",
            schema.description
        );
    }
}
