use orbit_types::{InvocationTrace, OrbitError};

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
