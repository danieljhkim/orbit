use orbit_common::types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{OrbitBuiltinAction, Tool, ToolContext};

pub struct OrbitAutoTaskAddTool;

impl Tool for OrbitAutoTaskAddTool {
    fn schema(&self) -> ToolSchema {
        let parameters = vec![
            ToolParam {
                name: "name".to_string(),
                description: "Unique definition name (lowercase alphanumeric, `-`/`_`, starts alphanumeric). Required.".to_string(),
                param_type: "string".to_string(),
                required: true,
            },
            ToolParam {
                name: "description".to_string(),
                description: "Human description of the recurring chore.".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
            ToolParam {
                name: "schedule".to_string(),
                description: "Schedule object: exactly one of `{ cron: string }` (5-field cron) or `{ every_minutes: number }`. Required.".to_string(),
                param_type: "object".to_string(),
                required: true,
            },
            ToolParam {
                name: "template".to_string(),
                description: "Task template: `{ title, description?, acceptance_criteria?, task_type?, tags?, priority?, crew?, status? }`. Provider-neutral — no turn knobs. Required.".to_string(),
                param_type: "object".to_string(),
                required: true,
            },
            ToolParam {
                name: "dedupe".to_string(),
                description: "Dedupe policy: `skip_if_open` (default) or `always`.".to_string(),
                param_type: "string".to_string(),
                required: false,
            },
        ];
        ToolSchema {
            name: "orbit.auto_task.add".to_string(),
            description: "Create a recurring auto-task definition. The generic scheduler mints a task from its template on each due slot.".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        super::super::execute_host_action(ctx, input, OrbitBuiltinAction::AutoTaskAdd)
    }
}
