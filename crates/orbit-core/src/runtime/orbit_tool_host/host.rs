use std::sync::Arc;

use orbit_common::types::OrbitError;
use orbit_tools::{OrbitBuiltinAction, OrbitTaskScope, OrbitToolHost, ReservationOwnerContext};
use serde_json::Value;

use crate::OrbitRuntime;

pub(crate) fn build_orbit_tool_host(
    runtime: &OrbitRuntime,
    task_id: Option<String>,
    run_id: Option<String>,
) -> Arc<dyn OrbitToolHost> {
    Arc::new(RuntimeOrbitToolHost {
        runtime: runtime.clone(),
        task_scope: OrbitTaskScope {
            orbit_root: Some(runtime.data_root_path().to_path_buf()),
            task_id,
            run_id: run_id.or_else(trusted_env_run_id),
        },
    })
}

#[derive(Clone)]
struct RuntimeOrbitToolHost {
    runtime: OrbitRuntime,
    task_scope: OrbitTaskScope,
}

impl OrbitToolHost for RuntimeOrbitToolHost {
    fn execute(
        &self,
        action: OrbitBuiltinAction,
        input: Value,
        agent: Option<String>,
        model: Option<String>,
        reservation_owner: Option<ReservationOwnerContext>,
    ) -> Result<Value, OrbitError> {
        let (agent, model) = self
            .runtime
            .try_canonical_agent_model_identity(agent.as_deref(), model.as_deref())?;
        super::dispatch::execute(
            &self.runtime,
            &self.task_scope,
            action,
            input,
            agent,
            model,
            reservation_owner,
        )
    }

    fn task_scope(&self) -> OrbitTaskScope {
        self.task_scope.clone()
    }
}

fn trusted_env_run_id() -> Option<String> {
    let managed = std::env::var("ORBIT_MANAGED_RUN_CONTEXT")
        .ok()
        .is_some_and(|value| matches!(value.trim(), "1" | "true" | "TRUE"));
    if !managed {
        return None;
    }
    std::env::var("ORBIT_RUN_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
