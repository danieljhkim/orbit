use orbit_exec::{ExecRequest, NoSandbox, run_process};
use orbit_types::{OrbitError, ToolSchema};
use serde_json::{Value, json};

use crate::{Tool, ToolContext};

pub struct OrbitJobRunArchiveTool;

pub(super) fn build_exec_request(
    ctx: &ToolContext,
    input: &Value,
) -> Result<ExecRequest, OrbitError> {
    let identity = super::resolve_identity(ctx, input)?;
    let id = super::required_string(input, &["id", "run_id", "runId"], "id")?;
    Ok(super::orbit_exec_request_with_identity(
        ctx,
        vec!["job-run".to_string(), "archive".to_string(), id],
        &identity,
    ))
}

impl Tool for OrbitJobRunArchiveTool {
    fn schema(&self) -> ToolSchema {
        let mut parameters = super::orbit_id_params("job run");
        parameters.extend(super::identity_params());
        ToolSchema {
            name: "orbit.job_run.archive".to_string(),
            description: "Archive an Orbit job run".to_string(),
            parameters,
            builtin: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, input: Value) -> Result<Value, OrbitError> {
        let id = super::required_string(&input, &["id", "run_id", "runId"], "id")?;
        let req = build_exec_request(ctx, &input)?;
        let result = run_process(&req, &NoSandbox)?;
        if !result.success {
            let stderr = result.stderr.trim();
            let detail = if stderr.is_empty() {
                "command returned non-zero exit status"
            } else {
                stderr
            };
            return Err(OrbitError::Execution(format!(
                "orbit job-run archive failed: {detail}"
            )));
        }

        Ok(json!({
            "archived": true,
            "id": id,
        }))
    }
}
