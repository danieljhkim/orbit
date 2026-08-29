//! Argument surface and seed inputs for `orbit mcp callers`.

use clap::Parser;

use super::*;

/// Minimal parser so this subcommand's arguments can be exercised without the
/// whole CLI tree.
#[derive(Parser)]
struct CallersCli {
    #[command(subcommand)]
    command: CallersSubcommand,
}

fn parse(argv: &[&str]) -> CallersSubcommand {
    CallersCli::parse_from(argv).command
}

#[test]
fn check_takes_the_machine_id_positionally() {
    match parse(&["callers", "check", "hm_alpha"]) {
        CallersSubcommand::Check(args) => assert_eq!(args.machine_id, "hm_alpha"),
        _ => panic!("expected `check`"),
    }
}

#[test]
fn check_requires_a_machine_id() {
    assert!(
        CallersCli::try_parse_from(["callers", "check"]).is_err(),
        "checking nothing would print the file default and read as an answer about a caller"
    );
}

#[test]
fn init_takes_no_grant_writing_flags() {
    // Granting operator is a hand edit by design; a flag for it here would
    // re-create the caller-authored grant the file replaces [ORB-11052].
    for argv in [
        ["callers", "init", "--operator"].as_slice(),
        ["callers", "init", "--grant", "operator"].as_slice(),
    ] {
        assert!(
            CallersCli::try_parse_from(argv).is_err(),
            "unexpected grant-writing flag accepted: {argv:?}"
        );
    }
}

#[test]
fn a_workspace_root_override_is_refused() {
    let args = CallersArgs {
        command: parse(&["callers", "list"]),
    };

    let error = args
        .execute_without_runtime(Some(Path::new("/tmp")))
        .expect_err("the callers file is machine-global");

    assert!(matches!(error, OrbitError::InvalidInput(_)), "{error:?}");
}

#[test]
fn seeding_reads_registry_owners_and_configured_destinations() {
    let root = tempfile::tempdir().expect("global root");
    orbit_registry::ensure_host_identity(root.path(), || {
        Ok(orbit_registry::NewHostIdentity {
            host_id: "this-host".to_string(),
            task_prefix: "TH".to_string(),
        })
    })
    .expect("host identity");
    let local_machine_id =
        match orbit_registry::inspect_host_identity(root.path()).expect("host identity state") {
            orbit_registry::HostIdentityState::Present(identity) => identity.machine_id,
            other => panic!("expected a present host identity, got {other:?}"),
        };
    std::fs::write(
        root.path().join("mcp-destinations.toml"),
        "[[destinations]]\nssh = \"alpha-box\"\nmachine_id = \"hm_alpha\"\n",
    )
    .expect("write destinations");

    let seeded = known_callers(root.path()).expect("seed inputs");

    let machine_ids: Vec<_> = seeded
        .iter()
        .map(|caller| caller.machine_id.as_str())
        .collect();
    assert!(machine_ids.contains(&"hm_alpha"));
    assert!(
        !machine_ids.contains(&local_machine_id.as_str()),
        "a row for this machine would authorize nothing it does not already resolve locally"
    );
    assert_eq!(
        seeded
            .iter()
            .find(|caller| caller.machine_id == "hm_alpha")
            .and_then(|caller| caller.label.as_deref()),
        Some("alpha-box"),
        "the configured SSH target is the operator-facing name already written down"
    );
}
