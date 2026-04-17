use orbit_engine::{AgentProtocolHost, ExecutionContext};
use orbit_types::OrbitError;
use serde_json::Value;

use super::envelope::{build_agent_stdin_envelope_payload, execute_commit_request_if_present};
use crate::OrbitRuntime;

impl AgentProtocolHost for OrbitRuntime {
    fn build_agent_stdin_envelope_payload(
        &self,
        execution: &ExecutionContext,
    ) -> Result<Vec<u8>, OrbitError> {
        build_agent_stdin_envelope_payload(self, execution)
    }

    fn execute_commit_request_if_present(&self, result: &Value) -> Result<(), OrbitError> {
        execute_commit_request_if_present(self, result)
    }
}
