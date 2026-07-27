use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitAdrUpdateTool;

impl Tool for OrbitAdrUpdateTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = vec![
            ToolParam {
                name: "id".to_string(),
                description: "Canonical ADR ID to update.".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "title".to_string(),
                description: "New title.".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "owner".to_string(),
                description: "New owner.".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "body".to_string(),
                description: "Replacement body markdown.".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "status".to_string(),
                description:
                    "New status. `proposed -> accepted` requires non-empty `related_tasks` on the resulting record. `accepted -> proposed` is rejected. Direct writes to `superseded` are rejected — use orbit.adr.supersede."
                        .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "related_features".to_string(),
                description: "Replacement related_features list.".to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
            ToolParam {
                name: "related_tasks".to_string(),
                description:
                    "Replacement related_tasks list. Required (non-empty) when transitioning proposed -> accepted unless the existing record already has tasks."
                        .to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
            ToolParam {
                name: "tags".to_string(),
                description:
                    "Replacement free-form ADR labels. Empty list clears existing tags; absence leaves them unchanged."
                        .to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
            ToolParam {
                name: "paths".to_string(),
                description:
                    "Replacement repo-relative applicability globs. Empty list clears existing paths; absence leaves them unchanged."
                        .to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
            ToolParam {
                name: "supersedes".to_string(),
                description: "Replacement supersedes list.".to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
            ToolParam {
                name: "legacy_ids".to_string(),
                description: "Replacement legacy_ids list (supports rollup aliasing).".to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
            ToolParam {
                name: "validation_warnings".to_string(),
                description: "Replacement validation_warnings list.".to_string(),
                param_type: "string_list".to_string(),
                required: false,
            },
            ToolParam {
                name: "legacy_validation".to_string(),
                description: "Set legacy_validation to `none` or `warned`.".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
        ];
        parameters.extend(super::super::model_identity_params());
        ToolSchema {
            name: "orbit.adr.update".to_string(),
            description: "Partial update of an ADR's mutable fields and/or status. Status transitions follow the ADR lifecycle rules."
                .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::reject_agent_field(&input, "orbit.adr.update")?;
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::AdrUpdate)
    }
}
