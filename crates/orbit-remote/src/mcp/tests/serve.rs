#![allow(missing_docs)]

use std::net::SocketAddr;

use orbit_common::types::McpCapability;

use super::super::{check_bindable_mcp_host, least_privileged_mcp_capability};

#[test]
fn loopback_addresses_pass_the_bind_guard() {
    let v4: SocketAddr = "127.0.0.1:0".parse().expect("v4 loopback");
    let v6: SocketAddr = "[::1]:0".parse().expect("v6 loopback");

    assert!(check_bindable_mcp_host(v4).is_ok());
    assert!(check_bindable_mcp_host(v6).is_ok());
}

#[test]
fn non_loopback_addresses_are_refused_by_the_pure_guard() {
    let routable: SocketAddr = "0.0.0.0:4000".parse().expect("non-loopback");

    let error = check_bindable_mcp_host(routable).expect_err("non-loopback must be refused");

    let message = error.to_string();
    assert!(
        message.contains("non-loopback"),
        "message should name the refusal: {message}"
    );
    assert!(
        message.contains("4000"),
        "message should carry the requested port for the suggested tunnel command: {message}"
    );
}

#[test]
fn an_unspecified_capability_resolves_to_the_least_privileged_default() {
    assert_eq!(
        least_privileged_mcp_capability(None).expect("agent floor"),
        McpCapability::Agent,
        "a deployment that says nothing must not become an operator"
    );
}

#[test]
fn an_explicit_capability_request_is_never_widened() {
    assert_eq!(
        least_privileged_mcp_capability(Some(McpCapability::Operator)).expect("operator"),
        McpCapability::Operator
    );
    assert_eq!(
        least_privileged_mcp_capability(Some(McpCapability::Agent)).expect("agent"),
        McpCapability::Agent
    );
}

/// ORB-10727 [ADR-0358]: `runner` is withdrawn from the bridge. No canonical
/// tool policy admits it, so such a session would advertise an empty surface —
/// refuse it by name instead of silently serving nothing.
#[test]
fn a_withdrawn_capability_request_is_refused_by_name() {
    let error = least_privileged_mcp_capability(Some(McpCapability::Runner))
        .expect_err("runner is not a v1 bridge capability");
    let message = error.to_string();
    assert!(
        message.contains("runner"),
        "names the capability: {message}"
    );
    assert!(
        message.contains("agent or operator"),
        "names the alternatives: {message}"
    );
}
