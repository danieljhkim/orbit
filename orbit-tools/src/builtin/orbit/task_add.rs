use orbit_exec::ExecRequest;
use orbit_types::{OrbitError, ToolParam, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitTaskAddTool;

pub(super) fn build_exec_request(
    ctx: &ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
    let title = super::required_string(input, &["title"], "title")?;
    let description = super::required_string(input, &["description"], "description")?;
    let plan = super::required_string(input, &["plan"], "plan")?;
    let workspace = super::required_string(input, &["workspace"], "workspace")?;

    let mut args = vec![
        "task".to_string(),
        "add".to_string(),
        "--title".to_string(),
        title,
        "--description".to_string(),
        description,
        "--plan".to_string(),
        plan,
        "--workspace".to_string(),
        workspace,
    ];

    if let Some(comment) = super::optional_string(input, "comment")? {
        args.push("--comment".to_string());
        args.push(comment);
    }
    if let Some(context) = super::optional_string(input, "context")? {
        args.push("--context".to_string());
        args.push(context);
    }
    if let Some(priority) = super::optional_string(input, "priority")? {
        args.push("--priority".to_string());
        args.push(priority);
    }
    if let Some(task_type) =
        super::optional_string_alias(input, &["type", "task_type", "taskType"])?
    {
        args.push("--type".to_string());
        args.push(task_type);
    }

    args.push("--json".to_string());
    Ok(super::orbit_exec_request(ctx, args))
}

impl Tool for OrbitTaskAddTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.task.add".to_string(),
            description: "Create an Orbit task and return the created task JSON".to_string(),
            parameters: vec![
                ToolParam {
                    name: "title".to_string(),
                    description: "Task title".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "description".to_string(),
                    description: "Task description markdown".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "plan".to_string(),
                    description: "Task plan markdown".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "workspace".to_string(),
                    description: "Workspace path for the task".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                },
                ToolParam {
                    name: "comment".to_string(),
                    description: "Optional initial task comment".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "context".to_string(),
                    description: "Optional comma-separated context file paths".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "priority".to_string(),
                    description: "Optional priority level".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
                ToolParam {
                    name: "type".to_string(),
                    description: "Optional task type".to_string(),
                    param_type: "string".to_string(),
                    required: false,
                },
            ],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(ctx, &input)?;
        super::run_orbit_json_command(ctx, req.args, "orbit task add")
    }
}
