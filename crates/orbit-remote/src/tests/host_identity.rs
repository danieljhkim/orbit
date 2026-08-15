use orbit_common::types::HOST_IDENTITY_SCHEMA_VERSION;

use crate::host_identity::{
    HostIdentityOutcome, HostIdentityState, NewHostIdentity, ensure_host_identity,
    inspect_host_identity, load_host_identity, rename_current_host_identity,
    rename_current_host_identity_with_writer,
};

fn requested(
    name: &str,
    task_prefix: &str,
) -> impl FnOnce() -> Result<NewHostIdentity, orbit_common::types::OrbitError> {
    let name = name.to_string();
    let task_prefix = task_prefix.to_string();
    move || {
        Ok(NewHostIdentity {
            host_id: name,
            task_prefix,
        })
    }
}

#[test]
fn create_persists_current_schema_identity() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outcome = ensure_host_identity(dir.path(), requested("dk-server-1", "DE")).expect("create");
    assert!(matches!(outcome, HostIdentityOutcome::Created(_)));
    let identity = outcome.identity();
    assert_eq!(identity.schema_version, HOST_IDENTITY_SCHEMA_VERSION);
    assert_eq!(identity.host_id, "dk-server-1");
    assert_eq!(identity.task_prefix, "DE");
    assert!(identity.machine_id.starts_with("hm_"));

    let loaded = load_host_identity(dir.path()).expect("load");
    assert_eq!(&loaded, identity);
}

#[test]
fn repeated_init_is_unchanged_and_stable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let created = ensure_host_identity(dir.path(), requested("dk-server-1", "DK")).expect("create");
    let first_machine_id = created.identity().machine_id.clone();
    let before = std::fs::read_to_string(dir.path().join("host.toml")).expect("read");

    // A second init must not prompt (the closure would panic) and must not
    // change the file.
    let again = ensure_host_identity(dir.path(), || panic!("must not create on repeat"))
        .expect("repeat init");
    assert!(matches!(again, HostIdentityOutcome::Unchanged(_)));
    assert_eq!(again.identity().machine_id, first_machine_id);
    assert_eq!(again.identity().task_prefix, "DK");
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
    assert_eq!(identity.task_prefix, "ORB");
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
fn schema_v1_identity_with_existing_sequence_migrates_preserving_machine_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("tasks")).expect("create task state");
    std::fs::write(dir.path().join("tasks/index.sqlite"), []).expect("seed task sequence");
    std::fs::write(
        dir.path().join("host.toml"),
        "schema_version = 1\nmachine_id = \"hm_existing\"\nhost_id = \"dk-server-1\"\nmode = \"hub\"\n",
    )
    .expect("seed schema v1 identity");

    let migrated =
        ensure_host_identity(dir.path(), || panic!("migration must not prompt")).expect("migrate");
    assert!(matches!(migrated, HostIdentityOutcome::Migrated(_)));
    assert_eq!(migrated.identity().machine_id, "hm_existing");
    assert_eq!(migrated.identity().task_prefix, "ORB");

    let path = dir.path().join("host.toml");
    let before = std::fs::read(&path).expect("read migrated identity");
    let text = std::str::from_utf8(&before).expect("host identity UTF-8");
    assert!(text.contains("schema_version = 2"), "{text}");
    assert!(text.contains("task_prefix = \"ORB\""), "{text}");
    assert!(!text.contains("mode ="), "{text}");

    let repeated =
        ensure_host_identity(dir.path(), || panic!("repeat must not prompt")).expect("repeat");
    assert!(matches!(repeated, HostIdentityOutcome::Unchanged(_)));
    assert_eq!(std::fs::read(path).expect("reread"), before);
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
        "schema_version = 2\nhost_id = \"dk-server-1\"\ntask_prefix = \"DE\"\n",
    )
    .expect("write");
    let error = inspect_host_identity(dir.path()).expect_err("missing machine_id must fail");
    assert!(error.to_string().contains("machine_id"), "{error}");
}

#[test]
fn schema_v2_identity_is_accepted_as_the_current_on_disk_format() {
    assert_eq!(HOST_IDENTITY_SCHEMA_VERSION, 2);
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("host.toml"),
        "schema_version = 2\nmachine_id = \"hm_current\"\nhost_id = \"dk-server-1\"\ntask_prefix = \"DE\"\n",
    )
    .expect("write schema-v2 identity");

    let identity = load_host_identity(dir.path()).expect("schema v2 must load");
    assert_eq!(identity.schema_version, 2);
    assert_eq!(identity.machine_id, "hm_current");
    assert_eq!(identity.host_id, "dk-server-1");
    assert_eq!(identity.task_prefix, "DE");
}

#[test]
fn transport_shaped_machine_id_is_rejected_without_rewriting_host_identity() {
    for machine_id in ["dk1", "user@dk1", "ssh:dk1", "hm_ssh:dk1", "/tmp/hub"] {
        let dir = tempfile::tempdir().expect("tempdir");
        let body = format!(
            "schema_version = 2\nmachine_id = \"{machine_id}\"\nhost_id = \"hub\"\ntask_prefix = \"DE\"\n"
        );
        let path = dir.path().join("host.toml");
        std::fs::write(&path, &body).expect("write hostile identity");
        let error = inspect_host_identity(dir.path())
            .expect_err("transport-shaped machine_id must fail")
            .to_string();
        assert!(error.contains("machine_id"), "unexpected: {error}");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read unchanged identity"),
            body
        );
    }
}

#[test]
fn blank_field_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("host.toml"),
        "schema_version = 2\nmachine_id = \"hm_1\"\nhost_id = \"  \"\ntask_prefix = \"DE\"\n",
    )
    .expect("write");
    inspect_host_identity(dir.path()).expect_err("blank host_id must fail closed");
}

#[test]
fn future_schema_version_fails_closed_without_rewrite() {
    let dir = tempfile::tempdir().expect("tempdir");
    let body = "schema_version = 3\nmachine_id = \"hm_future\"\nhost_id = \"dk-server-1\"\ntask_prefix = \"DE\"\n";
    std::fs::write(dir.path().join("host.toml"), body).expect("write");
    let error = inspect_host_identity(dir.path())
        .expect_err("future schema must fail closed")
        .to_string();
    assert!(error.contains("unsupported schema_version 3"), "{error}");
    assert!(error.contains("supports up to 2"), "{error}");
    assert!(error.contains("Upgrade Orbit"), "{error}");
    assert!(error.contains("file is left unchanged"), "{error}");
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
fn rename_current_identity_preserves_machine_id_and_task_prefix() {
    let dir = tempfile::tempdir().expect("tempdir");
    let created = ensure_host_identity(dir.path(), requested("old-name", "DE"))
        .expect("create")
        .identity()
        .clone();

    let renamed = rename_current_host_identity(dir.path(), "new-name").expect("rename");
    assert_eq!(renamed.host_id, "new-name");
    assert_eq!(renamed.machine_id, created.machine_id);
    assert_eq!(renamed.task_prefix, "DE");

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
    ensure_host_identity(dir.path(), requested("start", "DE")).expect("create");

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

#[test]
fn rename_current_identity_classifies_preserved_and_durability_uncertain_errors() {
    let dir = tempfile::tempdir().expect("tempdir");
    ensure_host_identity(dir.path(), requested("old", "DE")).expect("create");
    let before = std::fs::read(dir.path().join("host.toml")).expect("read before");

    let preserved = rename_current_host_identity_with_writer(dir.path(), "new", |_, _| {
        Err(std::io::Error::other("injected pre-rename failure"))
    })
    .expect_err("pre-rename failure surfaces")
    .to_string();
    assert!(preserved.contains("preserved"), "unexpected: {preserved}");
    assert_eq!(
        std::fs::read(dir.path().join("host.toml")).expect("read preserved"),
        before
    );

    let uncertain = rename_current_host_identity_with_writer(dir.path(), "new", |path, staged| {
        std::fs::write(path, staged)?;
        Err(std::io::Error::other("injected post-rename failure"))
    })
    .expect_err("post-rename error surfaces")
    .to_string();
    assert!(
        uncertain.contains("durability is uncertain"),
        "unexpected: {uncertain}"
    );
    let observed = load_host_identity(dir.path()).expect("read committed rename");
    assert_eq!(observed.host_id, "new");
}
