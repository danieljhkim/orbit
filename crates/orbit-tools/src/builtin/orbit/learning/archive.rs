use orbit_common::types::{OrbitError, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitLearningArchiveTool;

impl Tool for OrbitLearningArchiveTool {
    fn schema(&self) -> ToolSchema {
        let parameters = super::super::orbit_id_params("learning");
        ToolSchema {
            name: "orbit.learning.archive".to_string(),
            description:
                "Retire `id` without a replacement: flips status to `superseded` with `superseded_by: null`. Idempotent — archiving an already-superseded record is a no-op success. Use `orbit.learning.supersede` instead when a replacement learning exists."
                    .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::LearningArchive)
    }
}
