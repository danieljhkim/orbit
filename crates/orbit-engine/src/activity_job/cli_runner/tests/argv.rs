#![allow(missing_docs)]

use std::collections::HashMap;

use orbit_exec::sandbox_exec_program_for_audit;

use super::super::argv::{
    audit_argv_for_dispatch, neutralize_inner_sandbox, rewrite_debug_file_value,
};
use super::test_support::sandbox_for_test;

#[test]
fn audit_argv_for_dispatch_prepends_sandbox_exec_when_sandbox_active() {
    let argv = audit_argv_for_dispatch(
        "/usr/bin/claude",
        &["-p".to_string(), "hello".to_string()],
        Some(&sandbox_for_test()),
    );
    assert_eq!(
        argv,
        vec![
            sandbox_exec_program_for_audit(),
            "-f",
            "<profile.sb>",
            "/usr/bin/claude",
            "-p",
            "hello"
        ]
    );
}

#[test]
fn audit_argv_for_dispatch_returns_bare_when_no_sandbox() {
    let argv = audit_argv_for_dispatch(
        "/usr/bin/claude",
        &["-p".to_string(), "hello".to_string()],
        None,
    );
    assert_eq!(argv, vec!["/usr/bin/claude", "-p", "hello"]);
}

#[test]
fn neutralize_inner_sandbox_pins_codex_to_danger_full_access() {
    let mut config = HashMap::new();
    config.insert("sandbox".to_string(), "workspace-write".to_string());
    let mut args = vec!["exec".to_string(), "--json".to_string()];
    neutralize_inner_sandbox("codex", &mut config, &mut args);
    assert_eq!(
        config.get("sandbox").map(String::as_str),
        Some("danger-full-access"),
        "codex sandbox should be pinned to danger-full-access when outer sandbox is active"
    );
    // Static args are untouched for codex; the sandbox flag flows
    // through provider_config.
    assert_eq!(args, vec!["exec", "--json"]);
}

#[test]
fn neutralize_inner_sandbox_drops_gemini_sandbox_flags() {
    let mut config = HashMap::new();
    let mut args = vec![
        "--approval-mode".to_string(),
        "yolo".to_string(),
        "--sandbox".to_string(),
        "-s".to_string(),
        "-o".to_string(),
        "json".to_string(),
    ];
    neutralize_inner_sandbox("gemini", &mut config, &mut args);
    assert!(
        !args.iter().any(|a| a == "--sandbox" || a == "-s"),
        "gemini sandbox flags should be removed: {args:?}"
    );
    assert!(args.iter().any(|a| a == "--approval-mode"));
    assert!(args.iter().any(|a| a == "json"));
}

#[test]
fn neutralize_inner_sandbox_drops_grok_sandbox_flag_and_value() {
    let mut config = HashMap::new();
    let mut args = vec![
        "--permission-mode".to_string(),
        "bypassPermissions".to_string(),
        "--sandbox".to_string(),
        "workspace-write".to_string(),
        "--output-format".to_string(),
        "json".to_string(),
        "--sandbox=another-profile".to_string(),
    ];
    neutralize_inner_sandbox("grok", &mut config, &mut args);
    assert_eq!(
        args,
        vec![
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
            "--output-format".to_string(),
            "json".to_string(),
        ],
        "grok sandbox flags should be removed with their values"
    );
    assert!(
        config.is_empty(),
        "grok provider_config must remain untouched"
    );
}

#[test]
fn rewrite_debug_file_value_replaces_relative_path() {
    let mut args = vec![
        "-p".to_string(),
        "--debug-file".to_string(),
        ".orbit/state/logs/claude-debug.log".to_string(),
        "--tools".to_string(),
        "Read".to_string(),
    ];
    rewrite_debug_file_value(&mut args, std::path::Path::new("/Users/test/.claude"));
    assert_eq!(
        args,
        vec![
            "-p".to_string(),
            "--debug-file".to_string(),
            "/Users/test/.claude/claude-debug.log".to_string(),
            "--tools".to_string(),
            "Read".to_string(),
        ],
        "claude --debug-file value should be rewritten to <state_dir>/<basename>"
    );
}

#[test]
fn rewrite_debug_file_value_handles_bare_filename() {
    let mut args = vec!["--debug-file".to_string(), "claude-debug.log".to_string()];
    rewrite_debug_file_value(&mut args, std::path::Path::new("/Users/test/.claude"));
    assert_eq!(args[1], "/Users/test/.claude/claude-debug.log");
}

#[test]
fn rewrite_debug_file_value_no_op_without_flag() {
    let mut args = vec!["-p".to_string(), "--tools".to_string(), "Read".to_string()];
    let original = args.clone();
    rewrite_debug_file_value(&mut args, std::path::Path::new("/Users/test/.claude"));
    assert_eq!(
        args, original,
        "args without --debug-file should be untouched"
    );
}

#[test]
fn rewrite_debug_file_value_rewrites_every_occurrence() {
    let mut args = vec![
        "--debug-file".to_string(),
        "first.log".to_string(),
        "--other".to_string(),
        "x".to_string(),
        "--debug-file".to_string(),
        "nested/dir/second.log".to_string(),
    ];
    rewrite_debug_file_value(&mut args, std::path::Path::new("/Users/test/.claude"));
    assert_eq!(args[1], "/Users/test/.claude/first.log");
    assert_eq!(args[5], "/Users/test/.claude/second.log");
}

#[test]
fn rewrite_debug_file_value_falls_back_when_value_has_no_basename() {
    let mut args = vec!["--debug-file".to_string(), "/".to_string()];
    rewrite_debug_file_value(&mut args, std::path::Path::new("/Users/test/.claude"));
    assert_eq!(args[1], "/Users/test/.claude/claude-debug.log");
}

#[test]
fn rewrite_debug_file_value_ignores_dangling_flag() {
    let mut args = vec!["-p".to_string(), "--debug-file".to_string()];
    let original = args.clone();
    rewrite_debug_file_value(&mut args, std::path::Path::new("/Users/test/.claude"));
    assert_eq!(
        args, original,
        "trailing --debug-file with no value must not panic or rewrite"
    );
}

#[test]
fn neutralize_inner_sandbox_leaves_claude_args_unchanged() {
    let mut config = HashMap::new();
    let mut args = vec![
        "-p".to_string(),
        "--permission-mode".to_string(),
        "bypassPermissions".to_string(),
        "--tools".to_string(),
        "Read,Write,Edit,Bash".to_string(),
    ];
    let original = args.clone();
    neutralize_inner_sandbox("claude", &mut config, &mut args);
    assert_eq!(
        args, original,
        "claude args must be unchanged by neutralization"
    );
    assert!(
        config.is_empty(),
        "claude provider_config must remain untouched"
    );
}
