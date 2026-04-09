use orbit_types::{InvocationTrace, OrbitError};

use crate::types::{AgentRequest, AgentResponse};

pub trait AgentRuntime {
    fn invoke(&self, req: AgentRequest) -> Result<(AgentResponse, InvocationTrace), OrbitError>;

    fn model_name(&self) -> Option<&str> {
        None
    }
}
