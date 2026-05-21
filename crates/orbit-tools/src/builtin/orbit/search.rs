use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitSearchTool;

impl Tool for OrbitSearchTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = vec![
            ToolParam {
                name: "query".to_string(),
                description: "Free-text query. Defaults to lexical matching unless hybrid is true."
                    .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                // ADR-0179: expose the free-text vector ranker as hybrid, not semantic.
                name: "hybrid".to_string(),
                description:
                    "Opt into hybrid lexical + cosine ranking for indexed task and doc vectors; learnings and ADRs remain lexical."
                        .to_string(),
                param_type: "boolean".to_string(),
                required: false,
            },
            ToolParam {
                // ADR-0179: semantic carries the task ID for cosine-neighbor lookup on MCP.
                name: "semantic".to_string(),
                description:
                    "Task ID for cosine-neighbor lookup. Mutually exclusive with query."
                        .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "kind".to_string(),
                description: "Corpus kind: task, doc, learning, adr, or all. Default: all."
                    .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "limit".to_string(),
                description: "Maximum number of results. Default: 10.".to_string(),
                param_type: "integer".to_string(),
                required: false,
            },
            ToolParam {
                name: "tag".to_string(),
                description:
                    "AND-filter by tag. Repeat or pass an array. Applies to task, doc, learning, and ADR."
                        .to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
            ToolParam {
                name: "all".to_string(),
                description:
                    "Include normally-hidden statuses for the queried kind. Task adds done/rejected/archived; ADR adds superseded; learning adds superseded; doc is a no-op."
                        .to_string(),
                param_type: "boolean".to_string(),
                required: false,
            },
            ToolParam {
                name: "status".to_string(),
                description:
                    "Explicit per-kind status override using kind:value tokens, such as task:open,doc:active,adr:proposed. Overrides `all` for the named kind."
                        .to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
            ToolParam {
                name: "path".to_string(),
                description:
                    "Filter to artifacts applicable to this filesystem path. Task: selector containment. Learning and ADR: glob-containment over applicability globs. Doc out of scope (returns empty)."
                        .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
        ];
        parameters.extend(super::model_identity_params());
        ToolSchema {
            name: "orbit.search".to_string(),
            description:
                "Search tasks, docs, learnings, and ADRs. Hybrid vector ranking applies to indexed tasks and docs."
                    .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::execute_host_action(ctx, input, OrbitBuiltinAction::Search)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_schema_uses_hybrid_and_semantic_task_id_params() {
        let schema = OrbitSearchTool.schema();
        let params = schema
            .parameters
            .iter()
            .map(|param| (param.name.as_str(), param.param_type.as_str()))
            .collect::<Vec<_>>();

        assert!(params.contains(&("hybrid", "boolean")));
        assert!(params.contains(&("semantic", "string")));
        assert!(!params.iter().any(|(name, _)| *name == "related"));
        assert!(!params.iter().any(|(name, _)| *name == "field"));
        assert!(!params.iter().any(|(name, _)| *name == "embedding_model"));
    }
}
