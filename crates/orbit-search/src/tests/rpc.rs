//! Unit tests for the RPC envelope's boundary translator (ORB-10013) —
//! sibling layout.

use orbit_common::types::OrbitError;

use crate::rpc::{RpcError, rpc_error_to_orbit};

#[test]
fn rpc_error_to_orbit_renders_code_and_message_into_execution() {
    let error = RpcError {
        code: "embed_failed".to_string(),
        message: "model not loaded".to_string(),
    };
    assert!(matches!(
        rpc_error_to_orbit(error),
        OrbitError::Execution(m) if m == "search companion embed_failed: model not loaded"
    ));
}
