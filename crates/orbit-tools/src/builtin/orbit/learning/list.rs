use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitLearningListTool;

impl Tool for OrbitLearningListTool {
    fn schema(&self) -> ToolSchema {
        let parameters = vec![
            ToolParam {
                name: "status".to_string(),
                description: "Filter by status: `active` (default) or `superseded`.".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "tag".to_string(),
                description: "Filter to learnings whose `scope.tags` contains this tag.".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "path".to_string(),
                description:
                    "Filter to learnings whose `scope.paths` glob-contain this filesystem path. A learning matches when any of its scope globs resolves true against the given path."
                        .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "include_remote".to_string(),
                description:
                    "Include allocation rows whose body files are not locally readable as remote stubs."
                        .to_string(),
                param_type: "boolean".to_string(),
                required: false,
            },
        ];
        ToolSchema {
            name: "orbit.learning.list".to_string(),
            description:
                "List learnings ordered by `updated_at` desc. Returns envelope-only records; call `orbit.learning.show` for the full body."
                    .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::LearningList)
    }
}
