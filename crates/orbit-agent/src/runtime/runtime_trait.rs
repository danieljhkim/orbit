use orbit_common::OrbitError;
use orbit_types::telemetry::InvocationTrace;

use crate::types::{AgentInvocationSpec, AgentRequest};

pub trait AgentRuntime {
    fn invoke(
        &self,
        req: AgentRequest,
    ) -> Result<(AgentInvocationSpec, InvocationTrace), OrbitError>;

    fn model_name(&self) -> Option<&str> {
        None
    }
}
