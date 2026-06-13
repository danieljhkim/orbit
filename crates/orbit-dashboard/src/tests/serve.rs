use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use orbit_core::OrbitError;

use super::super::check_bindable_host;

#[test]
fn allows_ipv4_loopback() {
    let host = IpAddr::V4(Ipv4Addr::LOCALHOST);
    assert!(check_bindable_host(host, 7878).is_ok());
}

#[test]
fn allows_ipv6_loopback() {
    let host = IpAddr::V6(Ipv6Addr::LOCALHOST);
    assert!(check_bindable_host(host, 7878).is_ok());
}

#[test]
fn allows_127_0_0_x_range() {
    // The whole 127.0.0.0/8 block is loopback.
    let host = IpAddr::V4(Ipv4Addr::new(127, 5, 5, 5));
    assert!(check_bindable_host(host, 7878).is_ok());
}

#[test]
fn rejects_unspecified_address() {
    // `--host 0.0.0.0` is the exact exposure the guard exists to block.
    let host = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
    let err = check_bindable_host(host, 7878).expect_err("0.0.0.0 must be rejected");
    assert!(matches!(err, OrbitError::InvalidInput(_)));
}

#[test]
fn rejects_lan_address() {
    let host = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50));
    let err = check_bindable_host(host, 7878).expect_err("LAN address must be rejected");
    assert!(matches!(err, OrbitError::InvalidInput(_)));
}
