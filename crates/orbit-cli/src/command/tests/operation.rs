use clap::Parser;

use super::super::{Cli, operation::RuntimeNeed};

fn operation_for(args: &[&str]) -> super::super::operation::CommandOperation {
    Cli::parse_from(args).command.operation()
}

#[test]
fn runtime_free_command_set_is_derived_from_operations() {
    let runtime_free: &[&[&str]] = &[
        &["orbit", "init"],
        &["orbit", "workspace", "init"],
        &["orbit", "mcp", "init"],
        &["orbit", "mcp", "remove"],
        &["orbit", "mcp", "serve"],
        &["orbit", "migrate"],
        &["orbit", "migrate", "--dry-run"],
        &["orbit", "learning", "migrate-layout"],
        &["orbit", "run", "ship-sweep", "--dry-run"],
        &["orbit", "sweep", "--dry-run"],
        &["orbit", "routine", "list"],
        &["orbit", "web", "serve", "--no-open"],
        &["orbit", "web", "connect", "example.test", "--no-open"],
    ];

    for args in runtime_free {
        assert_eq!(
            operation_for(args).runtime_need,
            RuntimeNeed::Forbidden,
            "{args:?} must not bootstrap a workspace runtime"
        );
    }

    let runtime_required: &[&[&str]] = &[
        &["orbit", "workspace", "list"],
        &["orbit", "migrate", "--confirm"],
        &["orbit", "learning", "list"],
        &["orbit", "run", "history"],
        &["orbit", "task", "list"],
    ];
    for args in runtime_required {
        assert_eq!(
            operation_for(args).runtime_need,
            RuntimeNeed::Required,
            "{args:?} must bootstrap a workspace runtime"
        );
    }
}

#[test]
fn migrate_only_bootstraps_the_applying_form() {
    assert_eq!(
        operation_for(&["orbit", "migrate"]).runtime_need,
        RuntimeNeed::Forbidden
    );
    assert_eq!(
        operation_for(&["orbit", "migrate", "--dry-run"]).runtime_need,
        RuntimeNeed::Forbidden
    );
    assert_eq!(
        operation_for(&["orbit", "migrate", "--confirm"]).runtime_need,
        RuntimeNeed::Required
    );
}

#[test]
fn json_error_preferences_are_derived_from_operations() {
    assert_eq!(
        operation_for(&["orbit", "tool", "run", "orbit.task.show"]).json_error_preference,
        Some(false)
    );
    assert_eq!(
        operation_for(&["orbit", "tool", "run", "orbit.task.show", "--pretty",])
            .json_error_preference,
        Some(true)
    );
    assert_eq!(
        operation_for(&[
            "orbit",
            "tool",
            "run",
            "orbit.task.show",
            "--output",
            "text",
        ])
        .json_error_preference,
        None
    );
    assert_eq!(
        operation_for(&["orbit", "docs", "list", "--json"]).json_error_preference,
        Some(true)
    );
    assert_eq!(
        operation_for(&["orbit", "friction", "list", "--json"]).json_error_preference,
        Some(true)
    );
    assert_eq!(
        operation_for(&["orbit", "search", "registry", "--json"]).json_error_preference,
        Some(true)
    );
    assert_eq!(
        operation_for(&["orbit", "docs", "list"]).json_error_preference,
        None
    );
    assert_eq!(
        operation_for(&["orbit", "adr", "show", "ADR-0259", "--json"]).json_error_preference,
        Some(true)
    );
}

#[test]
fn adr_reconcile_operation_carries_target_and_json_preference() {
    let operation = operation_for(&[
        "orbit",
        "adr",
        "reconcile",
        "ADR-0184",
        "--source-worktree",
        "/tmp/source",
        "--json",
    ]);
    let meta = operation.audit_meta.expect("ADR reconcile audit metadata");
    assert_eq!(meta.command, "adr");
    assert_eq!(meta.subcommand.as_deref(), Some("reconcile"));
    assert_eq!(meta.target_id.as_deref(), Some("ADR-0184"));
    assert_eq!(operation.json_error_preference, Some(true));
}

#[test]
fn only_pretooluse_suppresses_runtime_and_command_errors() {
    assert!(operation_for(&["orbit", "hook", "pretooluse", "--format", "codex"]).suppress_errors);
    assert!(!operation_for(&["orbit", "hook", "install"]).suppress_errors);
    assert!(!operation_for(&["orbit", "task", "list"]).suppress_errors);
}

#[test]
fn audit_command_is_the_only_operation_without_audit_metadata() {
    assert!(
        operation_for(&["orbit", "audit", "list"])
            .audit_meta
            .is_none()
    );
    let meta = operation_for(&["orbit", "task", "show", "ORB-10200"])
        .audit_meta
        .expect("task show is audited");
    assert_eq!(meta.command, "task");
    assert_eq!(meta.subcommand.as_deref(), Some("show"));
    assert_eq!(meta.target_id.as_deref(), Some("ORB-10200"));

    let adr_meta = operation_for(&["orbit", "adr", "show", "ADR-0259"])
        .audit_meta
        .expect("ADR show is audited");
    assert_eq!(adr_meta.command, "adr");
    assert_eq!(adr_meta.subcommand.as_deref(), Some("show"));
    assert_eq!(adr_meta.target_id.as_deref(), Some("ADR-0259"));
}
