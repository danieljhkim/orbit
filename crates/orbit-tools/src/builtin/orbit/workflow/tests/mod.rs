use orbit_common::types::OrbitError;
use serde_json::json;
use std::sync::Arc;

use super::*;
use crate::{OrbitTaskScope, OrbitToolHost, ReservationOwnerContext};

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
        Ok(json!({"observed": true}))
    }

    fn task_scope(&self) -> OrbitTaskScope {
        OrbitTaskScope {
            run_id: Some("jrun-managed".to_string()),
            ..OrbitTaskScope::default()
        }
    }
}

fn managed_context() -> ToolContext {
    ToolContext {
        orbit_host: Some(Arc::new(ManagedHost)),
        ..ToolContext::default()
    }
}

#[test]
fn managed_run_rejects_dispatch_before_host_resolution() {
    let context = managed_context();

    let error = OrbitWorkflowShipTool
        .execute(&context, json!({"task_ids": ["ORB-00001"]}))
        .expect_err("managed run must not recursively dispatch");

    assert!(
        matches!(error, OrbitError::CapabilityDenied(_)),
        "{error:?}"
    );
    assert!(error.to_string().contains("managed runs cannot dispatch"));
}

#[test]
fn observation_remains_non_mutating_inside_managed_run() {
    let context = managed_context();

    let output = OrbitWorkflowRunShowTool
        .execute(&context, json!({"id": "jrun-example"}))
        .expect("observation remains available");

    assert_eq!(output, json!({"observed": true}));
}
