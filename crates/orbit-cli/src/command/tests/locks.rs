//! Parser-level coverage for `orbit task locks`.
//!
//! The reservation *decisions* belong to the domain and are tested there; what
//! this file pins is the argv contract the operator types, in particular the
//! "exactly one scope" rule that the tool states but that only the parser can
//! enforce before a runtime is built.

use clap::{Parser, error::ErrorKind};

use crate::command::locks::LocksSubcommand;
use crate::command::task::TaskSubcommand;
use crate::command::{Cli, Commands};

fn locks_subcommand(argv: &[&str]) -> LocksSubcommand {
    let mut full = vec!["orbit"];
    full.extend_from_slice(argv);
    let cli =
        Cli::try_parse_from(&full).unwrap_or_else(|err| panic!("{full:?} should parse: {err}"));
    let Commands::Task(task) = cli.command else {
        panic!("expected task command");
    };
    let TaskSubcommand::Locks(locks) = task.command else {
        panic!("expected task locks command");
    };
    locks.command
}

#[test]
fn locks_reserve_accepts_a_task_scope_and_a_file_scope() {
    let LocksSubcommand::Reserve(args) =
        locks_subcommand(&["task", "locks", "reserve", "--task", "ORB-00001,ORB-00002"])
    else {
        panic!("expected reserve");
    };
    assert_eq!(args.task_ids, ["ORB-00001", "ORB-00002"]);
    assert!(args.files.is_empty());
    assert_eq!(args.ttl, "30m");

    let LocksSubcommand::Reserve(args) = locks_subcommand(&[
        "task",
        "locks",
        "reserve",
        "--file",
        "dir:crates/orbit-cli",
        "--file",
        "file:README.md",
        "--ttl",
        "2h",
    ]) else {
        panic!("expected reserve");
    };
    assert_eq!(args.files, ["dir:crates/orbit-cli", "file:README.md"]);
    assert!(args.task_ids.is_empty());
    assert_eq!(args.ttl, "2h");
}

/// A reservation is atomic over one surface. Two scopes would be two surfaces,
/// and no scope would be an empty claim that always succeeds — both are
/// rejected before a runtime is built.
#[test]
fn locks_reserve_requires_exactly_one_scope() {
    let err = match Cli::try_parse_from(["orbit", "task", "locks", "reserve"]) {
        Ok(_) => panic!("reserve without a scope should not parse"),
        Err(err) => err,
    };
    assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);

    let err = match Cli::try_parse_from([
        "orbit",
        "task",
        "locks",
        "reserve",
        "--task",
        "ORB-00001",
        "--file",
        "file:src/lib.rs",
    ]) {
        Ok(_) => panic!("reserve with both scopes should not parse"),
        Err(err) => err,
    };
    assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
}

/// `release` still takes a reservation id positionally and still demands
/// `--confirm`; adding `reserve` next to it must not have moved either.
#[test]
fn locks_release_keeps_its_confirmed_positional_form() {
    let LocksSubcommand::Release(args) =
        locks_subcommand(&["task", "locks", "release", "reservation-1", "--confirm"])
    else {
        panic!("expected release");
    };
    assert_eq!(args.reservation_id, "reservation-1");
    assert!(args.confirm);
}
