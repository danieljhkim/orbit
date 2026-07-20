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
        &["orbit", "migrate", "--dry-run"],
        &["orbit", "learning", "migrate-layout"],
        &[
            "orbit",
            "knowledge",
            "allocate",
            "--kind",
            "adr",
            "--workspace",
            "ws_alpha",
        ],
        &["orbit", "knowledge", "sync", "--workspace", "/tmp/worktree"],
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
        &["orbit", "migrate"],
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
}
