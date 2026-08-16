//! [ORB-10711, ADR-0351] The self-dispatch guard on `orbit.command.exec`.
//!
//! Mirrors `orbit.workflow.ship`'s guard test: a managed run's leaf agent must
//! not reach this tool, because it could otherwise invoke the CLI to bypass
//! every other tool-specific policy. The guard reads `task_scope().run_id`
//! before the host is ever resolved, so a mock host is enough to prove it —
//! no runtime, claim, or process spawn required.

use orbit_common::types::OrbitError;
use serde_json::json;
use std::sync::Arc;

use crate::builtin::orbit::command::OrbitCommandExecTool;
use crate::{
    OrbitBuiltinAction, OrbitTaskScope, OrbitToolHost, ReservationOwnerContext, Tool, ToolContext,
};

struct ManagedHost;

impl OrbitToolHost for ManagedHost {
    fn execute(
        &self,
        _action: OrbitBuiltinAction,
        _input: serde_json::Value,
        _agent: Option<String>,
        _model: Option<String>,
        _reservation_owner: Option<ReservationOwnerContext>,
    ) -> Result<serde_json::Value, OrbitError> {
        panic!("the guard must refuse before the host is ever reached");
    }

    fn task_scope(&self) -> OrbitTaskScope {
        OrbitTaskScope {
            run_id: Some("jrun-managed".to_string()),
            ..OrbitTaskScope::default()
        }
    }
}

#[test]
fn managed_run_rejects_command_exec_before_host_resolution() {
    let context = ToolContext {
        orbit_host: Some(Arc::new(ManagedHost)),
        ..ToolContext::default()
    };

    let error = OrbitCommandExecTool
        .execute(
            &context,
            json!({"argv": ["git", "status"], "working_directory": "/tmp"}),
        )
        .expect_err("managed run must not execute remote commands");

    assert!(
        matches!(error, OrbitError::CapabilityDenied(_)),
        "{error:?}"
    );
    assert!(error.to_string().contains("managed runs cannot execute"));
}

#[test]
fn schema_requires_argv_and_working_directory() {
    let schema = OrbitCommandExecTool.schema();
    let required: Vec<&str> = schema
        .parameters
        .iter()
        .filter(|parameter| parameter.required)
        .map(|parameter| parameter.name.as_str())
        .collect();

    assert!(required.contains(&"argv"));
    assert!(required.contains(&"working_directory"));
}
