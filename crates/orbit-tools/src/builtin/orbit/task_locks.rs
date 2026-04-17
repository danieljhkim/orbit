use orbit_exec::ExecRequest;
use orbit_types::{OrbitError, ToolSchema};
use serde_json::Value;

use crate::{Tool, ToolContext};

pub struct OrbitTaskLocksTool;

pub(super) fn build_exec_request(ctx: &ToolContext) -> ExecRequest {
    super::orbit_exec_request_with_identity(
        ctx,
        vec![
            "task".to_string(),
            "locks".to_string(),
            "--json".to_string(),
        ],
        &super::OrbitIdentity::default(),
    )
}

fn validate_output(output: &Value) -> Result<(), OrbitError> {
    let object = output.as_object().ok_or_else(|| {
        OrbitError::Execution(
            "failed to parse orbit task locks output: expected JSON object".to_string(),
        )
    })?;

    if !object.get("locked_files").is_some_and(Value::is_array) {
        return Err(OrbitError::Execution(
            "failed to parse orbit task locks output: missing array `locked_files`".to_string(),
        ));
    }
    if !object.get("by_task").is_some_and(Value::is_array) {
        return Err(OrbitError::Execution(
            "failed to parse orbit task locks output: missing array `by_task`".to_string(),
        ));
    }
    if !object
        .get("total_locked")
        .is_some_and(|value| value.is_u64() || value.is_i64())
    {
        return Err(OrbitError::Execution(
            "failed to parse orbit task locks output: missing integer `total_locked`".to_string(),
        ));
    }
    if !object
        .get("total_tasks")
        .is_some_and(|value| value.is_u64() || value.is_i64())
    {
        return Err(OrbitError::Execution(
            "failed to parse orbit task locks output: missing integer `total_tasks`".to_string(),
        ));
    }

    Ok(())
}

impl Tool for OrbitTaskLocksTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "orbit.task.locks".to_string(),
            description: "List files currently locked by active Orbit tasks as JSON.".to_string(),
            parameters: vec![],
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, _input: Value) -> Result<Value, OrbitError> {
        let req = build_exec_request(ctx);
        let output = super::run_orbit_json_command(req, "orbit task locks")?;
        validate_output(&output)?;
        Ok(output)
    }
}
