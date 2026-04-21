use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use orbit_knowledge::{Selector, TaskGraphService};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitKnowledgePackTool;

impl Tool for OrbitKnowledgePackTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.graph.pack".to_string(),
            description:
                "Resolve knowledge selectors into a scoped pack from `.orbit/knowledge` artifacts. `file:` selectors return file metadata and symbol summaries, not full file source."
                    .to_string(),
            parameters: vec![
                ToolParam {
                    name: "selectors".to_string(),
                    description: "Selector strings like `file:path`, `symbol:path#symbol:kind`, or `dir:path`. Use `orbit.graph.show` or `symbol:` selectors when you need file source.".to_string(),
                    param_type: "array".to_string(),
                    required: true,
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
        let selectors = parse_selector_strings(&input)?;
        let selectors = Selector::parse_many(&selectors)
            .map_err(|error| OrbitError::InvalidInput(error.to_string()))?;
        let knowledge_dir = super::knowledge_write::resolve_knowledge_dir(ctx, &input)?;
        let explicit_ref = super::optional_string(&input, "ref")?;
        let service =
            TaskGraphService::new(knowledge_dir, super::knowledge_write::task_graph_scope(ctx));
        service.pack_json(
            &selectors,
            ctx.workspace_root.as_deref(),
            super::has_explicit_knowledge_dir(&input),
            explicit_ref.as_deref(),
        )
    }
}

fn parse_selector_strings(input: &Value) -> Result<Vec<String>, OrbitError> {
    let raw = input
        .get("selectors")
        .ok_or_else(|| OrbitError::InvalidInput("missing `selectors`".to_string()))?;
    let items = raw
        .as_array()
        .ok_or_else(|| OrbitError::InvalidInput("`selectors` must be an array".to_string()))?;
    if items.is_empty() {
        return Err(OrbitError::InvalidInput(
            "`selectors` must contain at least one selector".to_string(),
        ));
    }

    items
        .iter()
        .map(|item| {
            item.as_str().map(str::to_string).ok_or_else(|| {
                OrbitError::InvalidInput("`selectors` entries must be strings".to_string())
            })
        })
        .collect()
}
