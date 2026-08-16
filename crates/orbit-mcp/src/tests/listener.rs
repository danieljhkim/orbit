//! Bind-policy tests for the MCP TCP listener.

use super::*;

fn addr(text: &str) -> SocketAddr {
    text.parse().expect("socket address")
}

#[test]
fn loopback_only_is_the_default_exposure() {
    assert_eq!(ListenerExposure::default(), ListenerExposure::LoopbackOnly);
}

#[test]
fn loopback_addresses_bind_under_the_default_exposure() {
    for candidate in ["127.0.0.1:0", "127.0.0.5:7879", "[::1]:0"] {
        assert!(
            ensure_bind_allowed(addr(candidate), ListenerExposure::LoopbackOnly).is_ok(),
            "{candidate} is loopback and must be allowed"
        );
    }
}

#[test]
fn non_loopback_addresses_are_refused_before_the_socket_opens() {
    let error = ensure_bind_allowed(addr("0.0.0.0:7879"), ListenerExposure::LoopbackOnly)
        .expect_err("wildcard bind must be refused");
    let OrbitError::InvalidInput(message) = error else {
        panic!("expected an invalid-input refusal");
    };
    assert!(message.contains("0.0.0.0:7879"), "{message}");
    assert!(message.contains("--allow-non-loopback"), "{message}");
}

#[test]
fn explicit_exposure_allows_a_non_loopback_bind() {
    assert!(ensure_bind_allowed(addr("0.0.0.0:7879"), ListenerExposure::AnyInterface).is_ok());
}
