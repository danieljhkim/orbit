use super::super::host::{
    HOST_IDENTITY_SCHEMA_VERSION, HostIdentityOutcome, HostIdentityState, HostMode,
    NewHostIdentity, ensure_host_identity, inspect_host_identity, load_host_identity,
    rename_current_host_identity,
};

fn requested(
    name: &str,
    mode: HostMode,
) -> impl FnOnce() -> Result<NewHostIdentity, orbit_common::types::OrbitError> {
    let name = name.to_string();
    move || {
        Ok(NewHostIdentity {
            host_id: name,
            mode,
        })
    }
}

#[test]
fn create_persists_current_schema_identity() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outcome = ensure_host_identity(dir.path(), requested("dk-server-1", HostMode::Standalone))
        .expect("create");
    assert!(matches!(outcome, HostIdentityOutcome::Created(_)));
    let identity = outcome.identity();
    assert_eq!(identity.schema_version, HOST_IDENTITY_SCHEMA_VERSION);
    assert_eq!(identity.host_id, "dk-server-1");
    assert_eq!(identity.mode, HostMode::Standalone);
    assert!(identity.machine_id.starts_with("hm_"));

    let loaded = load_host_identity(dir.path()).expect("load");
    assert_eq!(&loaded, identity);
}

#[test]
fn repeated_init_is_unchanged_and_stable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let created =
        ensure_host_identity(dir.path(), requested("dk-server-1", HostMode::Hub)).expect("create");
    let first_machine_id = created.identity().machine_id.clone();
    let before = std::fs::read_to_string(dir.path().join("host.toml")).expect("read");

    // A second init must not prompt (the closure would panic) and must not
    // change the file.
    let again = ensure_host_identity(dir.path(), || panic!("must not create on repeat"))
        .expect("repeat init");
    assert!(matches!(again, HostIdentityOutcome::Unchanged(_)));
    assert_eq!(again.identity().machine_id, first_machine_id);
    assert_eq!(again.identity().mode, HostMode::Hub);
    let after = std::fs::read_to_string(dir.path().join("host.toml")).expect("read");
    assert_eq!(before, after, "repeat init must not rewrite host.toml");
}

#[test]
fn legacy_host_id_only_file_migrates_idempotently() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("host.toml"), "host_id = \"dk-server-1\"\n").expect("write");

    assert!(matches!(
        inspect_host_identity(dir.path()).expect("inspect"),
        HostIdentityState::Legacy { .. }
    ));

    let migrated =
        ensure_host_identity(dir.path(), || panic!("migration must not prompt")).expect("migrate");
    assert!(matches!(migrated, HostIdentityOutcome::Migrated(_)));
    let identity = migrated.identity();
    assert_eq!(identity.schema_version, HOST_IDENTITY_SCHEMA_VERSION);
    assert_eq!(identity.host_id, "dk-server-1");
    assert_eq!(identity.mode, HostMode::Standalone);
    let machine_id = identity.machine_id.clone();

    // Second initialization preserves the generated machine_id and rewrites
    // nothing (identical file).
    let before = std::fs::read_to_string(dir.path().join("host.toml")).expect("read");
    let again = ensure_host_identity(dir.path(), || panic!("no create on repeat")).expect("repeat");
    assert!(matches!(again, HostIdentityOutcome::Unchanged(_)));
    assert_eq!(again.identity().machine_id, machine_id);
    let after = std::fs::read_to_string(dir.path().join("host.toml")).expect("read");
    assert_eq!(before, after);
}

#[test]
fn absent_file_loads_as_actionable_error_not_hostname() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(matches!(
        inspect_host_identity(dir.path()).expect("inspect"),
        HostIdentityState::Absent
    ));
    let error = load_host_identity(dir.path()).expect_err("absent must fail closed");
    assert!(error.to_string().contains("orbit init"), "{error}");
}

#[test]
fn legacy_file_loads_as_error_until_migrated() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("host.toml"), "host_id = \"dk-server-1\"\n").expect("write");
    let error = load_host_identity(dir.path()).expect_err("legacy must not load strictly");
    assert!(error.to_string().contains("migrate"), "{error}");
}

#[test]
fn malformed_toml_is_an_error_not_a_fallback() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("host.toml"), "host_id = [not toml").expect("write");
    inspect_host_identity(dir.path()).expect_err("invalid toml must fail closed");
}

#[test]
fn incomplete_current_schema_file_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("host.toml"),
        "schema_version = 1\nhost_id = \"dk-server-1\"\nmode = \"standalone\"\n",
    )
    .expect("write");
    let error = inspect_host_identity(dir.path()).expect_err("missing machine_id must fail");
    assert!(error.to_string().contains("machine_id"), "{error}");
}

#[test]
fn blank_field_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("host.toml"),
        "schema_version = 1\nmachine_id = \"hm_1\"\nhost_id = \"  \"\nmode = \"standalone\"\n",
    )
    .expect("write");
    inspect_host_identity(dir.path()).expect_err("blank host_id must fail closed");
}

#[test]
fn future_schema_version_fails_closed_without_rewrite() {
    let dir = tempfile::tempdir().expect("tempdir");
    let body = "schema_version = 2\nmachine_id = \"hm_future\"\nhost_id = \"dk-server-1\"\nmode = \"standalone\"\n";
    std::fs::write(dir.path().join("host.toml"), body).expect("write");
    inspect_host_identity(dir.path()).expect_err("future schema must fail closed");
    // The file is left untouched (never rewritten by a read).
    assert_eq!(
        std::fs::read_to_string(dir.path().join("host.toml")).expect("read"),
        body
    );
    // ensure must also refuse to overwrite it.
    ensure_host_identity(dir.path(), || panic!("must not create over a future file"))
        .expect_err("ensure must not overwrite future schema");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("host.toml")).expect("read"),
        body
    );
}

#[test]
fn invalid_mode_is_rejected() {
    assert!(HostMode::parse("standalone").is_ok());
    assert!(HostMode::parse("hub").is_ok());
    assert!(HostMode::parse("spoke").is_ok());
    HostMode::parse("bogus").expect_err("invalid mode must fail");
}

#[test]
fn rename_current_identity_preserves_machine_id_and_mode() {
    let dir = tempfile::tempdir().expect("tempdir");
    let created = ensure_host_identity(dir.path(), requested("old-name", HostMode::Hub))
        .expect("create")
        .identity()
        .clone();

    let renamed = rename_current_host_identity(dir.path(), "new-name").expect("rename");
    assert_eq!(renamed.host_id, "new-name");
    assert_eq!(renamed.machine_id, created.machine_id);
    assert_eq!(renamed.mode, HostMode::Hub);

    // The on-disk file reflects the rename and remains a complete identity.
    match inspect_host_identity(dir.path()).expect("inspect") {
        HostIdentityState::Present(identity) => {
            assert_eq!(identity.host_id, "new-name");
            assert_eq!(identity.machine_id, created.machine_id);
        }
        other => panic!("expected Present, got {other:?}"),
    }
}

#[test]
fn rename_current_identity_round_trips_quotes_and_rejects_absent_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    ensure_host_identity(dir.path(), requested("start", HostMode::Standalone)).expect("create");

    // A quote-bearing name must round-trip through the TOML staging rather than
    // corrupt the file.
    let quoted = rename_current_host_identity(dir.path(), "a\"b").expect("rename with quote");
    assert_eq!(quoted.host_id, "a\"b");
    match inspect_host_identity(dir.path()).expect("inspect") {
        HostIdentityState::Present(identity) => assert_eq!(identity.host_id, "a\"b"),
        other => panic!("expected Present, got {other:?}"),
    }

    // Renaming when no identity exists is a hard error, not a silent create.
    let empty = tempfile::tempdir().expect("tempdir");
    rename_current_host_identity(empty.path(), "whatever").expect_err("absent file must fail");
}
