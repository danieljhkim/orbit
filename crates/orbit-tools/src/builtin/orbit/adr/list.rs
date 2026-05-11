use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitAdrListTool;

impl Tool for OrbitAdrListTool {
    fn schema(&self) -> ToolSchema {
        let parameters = vec![
            ToolParam {
                name: "status".to_string(),
                description:
                    "Filter by status: `proposed`, `accepted`, `superseded`, or `deleted`."
                        .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "owner".to_string(),
                description: "Filter by owner (e.g. `claude`).".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "feature".to_string(),
                description:
                    "Filter by feature folder name appearing in `related_features` (e.g. `activity-job`)."
                        .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "task_id".to_string(),
                description: "Filter by Orbit task ID appearing in `related_tasks`.".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "legacy_id".to_string(),
                description:
                    "Filter by legacy ID alias appearing in `legacy_ids` (e.g. `activity-job/ADR-039`)."
                        .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "validation_warned".to_string(),
                description:
                    "When true, return only ADRs with `legacy_validation = warned`. When false, exclude them."
                        .to_string(),
                param_type: "boolean".to_string(),
                required: false,
            },
        ];
        ToolSchema {
            name: "orbit.adr.list".to_string(),
            description:
                "List ADRs filtered by status, owner, feature, task_id, legacy_id, or validation flag. Returns envelope-only records sorted descending by global ID."
                    .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::AdrList)
    }
}
