use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitLearningAckTool;

impl Tool for OrbitLearningAckTool {
    fn schema(&self) -> ToolSchema {
        let parameters = vec![
            ToolParam {
                name: "ids".to_string(),
                description:
                    "Learning ID or array of learning IDs to ack (e.g. the IDs listed in an injected reminder block)."
                        .to_string(),
                param_type: "array".to_string(),
                required: true,
            },
            ToolParam {
                name: "outcome".to_string(),
                description:
                    "`used` (default) when the learning shaped the work, `ignored` to record an explicit dismissal. Injections with no ack already count as ignored."
                        .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "session_id".to_string(),
                description:
                    "Session the ack belongs to. Defaults to `ORBIT_SESSION_ID` when exported."
                        .to_string(),
                param_type: "string".to_string(),
                required: false,
            },
        ];
        ToolSchema {
            name: "orbit.learning.ack".to_string(),
            description:
                "Record a used/ignored feedback ack for injected learnings. Feeds the per-learning usage rollup (`orbit learning stats`) that drives deprecation decisions."
                    .to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::LearningAck)
    }
}
