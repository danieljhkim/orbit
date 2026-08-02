#![allow(missing_docs)]

use std::collections::HashMap;
#[cfg(target_os = "linux")]
use std::process::Stdio;

#[cfg(target_os = "linux")]
use chrono::Utc;
#[cfg(target_os = "linux")]
use orbit_common::types::{
    ActivityV2Spec, ExecutorSandboxKind, FsOperation, PolicyDef, ResourceKind, load_activity_asset,
    parse_policy_resource,
};
use orbit_exec::sandbox_exec_program_for_audit;
#[cfg(target_os = "linux")]
use orbit_exec::{
    LinuxBwrapSpawnRequest, bwrap_program_for_audit, compile_linux_bwrap_argv, probe_bwrap,
    spawn_under_linux_bwrap,
};

#[cfg(target_os = "linux")]
use super::super::argv::try_audit_argv_for_dispatch;
use super::super::argv::{
    audit_argv_for_dispatch, neutralize_inner_sandbox, rewrite_debug_file_value,
};
use super::test_support::sandbox_for_test;
#[cfg(target_os = "linux")]
use crate::activity_job::ResolvedSandbox;

#[cfg(target_os = "linux")]
const TASK_PILOT_ACTIVITY: &str =
    include_str!("../../../../../orbit-core/assets/activities/task_pilot.yaml");
#[cfg(target_os = "linux")]
const DEFAULT_POLICY: &str = include_str!("../../../../../orbit-core/assets/policies/default.yaml");

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

#[cfg(target_os = "linux")]
#[test]
fn task_pilot_reviewer_profile_starts_direct_linux_invocation_with_env_denies() {
    let activity = load_activity_asset(TASK_PILOT_ACTIVITY).expect("parse task-pilot activity");
    let profile_name = activity
        .spec
        .fs_profile
        .as_deref()
        .expect("task-pilot must select an explicit fsProfile");
    assert_eq!(profile_name, "reviewer");
    let ActivityV2Spec::AgentLoop(agent) = &activity.spec.spec else {
        panic!("task-pilot must remain an agent-loop activity");
    };
    assert!(!agent.tools.iter().any(|tool| {
        matches!(
            tool.as_str(),
            "fs.write" | "fs.patch" | "fs.delete" | "orbit.task.update" | "orbit.task.*"
        )
    }));

    let resource =
        parse_policy_resource(DEFAULT_POLICY, "default task-pilot policy").expect("parse policy");
    assert_eq!(resource.kind, ResourceKind::Policy);
    assert_eq!(
        resource.spec.deny_read,
        ["**/.env", "**/.env.*", "**/*.env", "**/*.env.*"]
    );
    assert_eq!(
        resource.spec.deny_modify,
        [
            ".orbit/**",
            "!.orbit/auto_tasks/**",
            "!.orbit/routines/**",
            "!.orbit/config.yaml",
            "!.orbit/config.toml",
            "!.orbit/resources/**",
            "**/.env",
            "**/.env.*",
            "**/*.env",
            "**/*.env.*",
        ]
    );
    let now = Utc::now();
    let policy = PolicyDef {
        name: resource.metadata.name,
        description: resource.spec.description,
        deny_read: resource.spec.deny_read,
        deny_modify: resource.spec.deny_modify,
        fs_profiles: resource.spec.fs_profiles,
        created_at: now,
        updated_at: now,
    };
    assert!(
        policy
            .check_path(profile_name, FsOperation::Read, "src/lib.rs")
            .expect("check workspace read")
            .allowed
    );
    assert!(
        !policy
            .check_path(profile_name, FsOperation::Read, ".env")
            .expect("check env read")
            .allowed
    );
    for path in ["src/lib.rs", ".orbit/tasks/ORB-10584/task.yaml", ".env"] {
        assert!(
            !policy
                .check_path(profile_name, FsOperation::Modify, path)
                .unwrap_or_else(|error| panic!("check modify path {path}: {error}"))
                .allowed,
            "task-pilot reviewer profile must not modify {path}"
        );
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(workspace.join(".orbit")).expect("create workspace fixture");
    let workspace = workspace.canonicalize().expect("canonical workspace");
    let workspace_root = workspace.display().to_string();
    let mut effective = policy
        .effective_profile(profile_name)
        .expect("resolve reviewer profile");
    effective.read = effective
        .read
        .iter()
        .map(|rule| absolutize_test_rule(&workspace_root, rule))
        .collect();
    effective.modify = effective
        .modify
        .iter()
        .map(|rule| absolutize_test_rule(&workspace_root, rule))
        .collect();
    assert!(
        effective.modify.iter().all(|rule| rule.starts_with('!')),
        "reviewer profile must not contain a positive workspace modify grant: {:?}",
        effective.modify
    );

    let sandbox = ResolvedSandbox {
        kind: ExecutorSandboxKind::LinuxBwrap,
        fs_profile: effective.clone(),
        allow_fallback: false,
        managed_worktree: false,
    };
    let argv = try_audit_argv_for_dispatch("/bin/true", &[], Some(&sandbox), Some(&workspace))
        .expect("direct task-pilot Bubblewrap plan must compile");
    assert_eq!(
        argv.first().map(String::as_str),
        Some(bwrap_program_for_audit())
    );

    let probe = probe_bwrap();
    if !probe.available {
        return;
    }
    let plan = compile_linux_bwrap_argv(&effective, "/bin/true", &[], Some(&workspace), false)
        .expect("compile task-pilot Bubblewrap invocation");
    let mut child = spawn_under_linux_bwrap(LinuxBwrapSpawnRequest {
        plan: &plan,
        env: &[],
        cwd: Some(&workspace),
        stdin: Stdio::null(),
        stdout: Stdio::null(),
        stderr: Stdio::null(),
    })
    .expect("start direct task-pilot Bubblewrap invocation");
    assert!(
        child
            .wait()
            .expect("wait for task-pilot invocation")
            .success()
    );
}

#[cfg(target_os = "linux")]
fn absolutize_test_rule(workspace_root: &str, rule: &str) -> String {
    let (negated, body) = rule
        .strip_prefix('!')
        .map_or((false, rule), |body| (true, body));
    let body = body.trim_start_matches("./");
    let absolute = format!("{}/{body}", workspace_root.trim_end_matches('/'));
    if negated {
        format!("!{absolute}")
    } else {
        absolute
    }
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
