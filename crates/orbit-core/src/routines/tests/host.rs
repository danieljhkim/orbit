use super::super::host::{resolve_host_id, write_host_id};

#[test]
fn write_and_resolve_round_trip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_host_id(dir.path(), "dk-server-1").expect("write");
    assert!(path.ends_with("host.toml"));
    assert_eq!(resolve_host_id(dir.path()).expect("resolve"), "dk-server-1");
}

#[test]
fn missing_host_toml_falls_back_to_hostname() {
    let dir = tempfile::tempdir().expect("tempdir");
    let resolved = resolve_host_id(dir.path()).expect("resolve");
    assert!(!resolved.is_empty());
}

#[test]
fn empty_host_id_is_rejected_on_write() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_host_id(dir.path(), "  ").expect_err("empty host_id must fail");
}

#[test]
fn invalid_host_toml_is_an_error_not_a_fallback() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("host.toml"), "host_id = [not toml").expect("write");
    resolve_host_id(dir.path()).expect_err("invalid toml must fail closed");
}

#[test]
fn blank_host_id_key_falls_back_to_hostname() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("host.toml"), "host_id = \"  \"\n").expect("write");
    let resolved = resolve_host_id(dir.path()).expect("resolve");
    assert!(!resolved.is_empty());
}
