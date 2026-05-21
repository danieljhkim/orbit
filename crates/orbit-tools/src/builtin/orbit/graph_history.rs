//! `orbit.graph.history` compatibility tool.

use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_knowledge::Selector;
use orbit_knowledge::workflows::observe::REMOVED_GRAPH_HISTORY_MESSAGE;
use serde_json::Value;

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
mod tests;
