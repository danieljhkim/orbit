use std::collections::HashMap;

use orbit_common::types::ExecutorSandboxKind;
use orbit_exec::{claude_state_dir_from_env, sandbox_exec_program_for_audit};

use super::super::dispatcher::ResolvedSandbox;

/// Build the argv we audit-log. When wrapped, the parent process the kernel
/// sees is the trusted `sandbox-exec`, so we prepend
/// `<trusted sandbox-exec> -f <profile_path>` to
/// the child program. The profile path is the literal `<profile.sb>` because
/// the real path is a tempfile created at spawn time and only meaningful to
/// the kernel — the placeholder keeps the audit record stable across runs.
pub(super) fn audit_argv_for_dispatch(
    program: &str,
    args: &[String],
    sandbox: Option<&ResolvedSandbox>,
) -> Vec<String> {
    match sandbox {
        Some(sb) if sb.kind == ExecutorSandboxKind::MacosSandboxExec => {
            let mut out = Vec::with_capacity(args.len() + 4);
            out.push(sandbox_exec_program_for_audit().to_string());
            out.push("-f".to_string());
            out.push("<profile.sb>".to_string());
            out.push(program.to_string());
            out.extend(args.iter().cloned());
            out
        }
        _ => {
            let mut out = Vec::with_capacity(args.len() + 1);
            out.push(program.to_string());
            out.extend(args.iter().cloned());
            out
        }
    }
}

/// Pin codex's `--sandbox` to `danger-full-access` and drop gemini's `-s` /
/// `--sandbox` flag so the inner CLI sandbox does not double-encode the
/// outer orbit-exec sandbox. Claude has no native sandbox flag — nothing to
/// neutralize.
pub(super) fn neutralize_inner_sandbox(
    provider: &str,
    provider_config: &mut HashMap<String, String>,
    static_args: &mut Vec<String>,
) {
    match provider {
        "codex" => {
            provider_config.insert("sandbox".to_string(), "danger-full-access".to_string());
        }
        "gemini" => {
            *static_args = filter_gemini_inner_sandbox_args(static_args);
        }
        _ => {}
    }
}

/// Sandbox-orthogonal arg fixups the dispatcher applies before spawn. Today
/// this only normalizes Claude's `--debug-file` path so the log lands at a
/// sandbox-allowed location regardless of how the executor YAML spelled it.
pub(super) fn apply_provider_static_arg_fixups(provider: &str, static_args: &mut [String]) {
    if provider == "claude" {
        rewrite_claude_debug_file_path(static_args);
    }
}

/// Replace the value following any `--debug-file` token in `static_args`
/// with `<claude_state_dir>/<basename>`. Falls back to leaving the args
/// untouched when the state dir is unresolvable (e.g. `HOME` and
/// `CLAUDE_CONFIG_DIR` both unset) — the original relative path still
/// works in non-sandboxed runs, and the sandbox failure mode is what the
/// caller is opting into.
fn rewrite_claude_debug_file_path(static_args: &mut [String]) {
    let Some(state_dir) = claude_state_dir_from_env() else {
        return;
    };
    rewrite_debug_file_value(static_args, &state_dir);
}

fn rewrite_debug_file_value(static_args: &mut [String], state_dir: &std::path::Path) {
    let mut idx = 0;
    while idx + 1 < static_args.len() {
        if static_args[idx] == "--debug-file" {
            let basename = std::path::Path::new(&static_args[idx + 1])
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "claude-debug.log".to_string());
            static_args[idx + 1] = state_dir.join(basename).display().to_string();
            idx += 2;
        } else {
            idx += 1;
        }
    }
}

/// Strip gemini's sandbox flags from a static-args vector. `-s` and
/// `--sandbox` are toggle flags (no value); `--sandbox-image` would take a
/// value but gemini's sandbox-image is not currently used by orbit and is
/// out of scope.
fn filter_gemini_inner_sandbox_args(args: &[String]) -> Vec<String> {
    args.iter()
        .filter(|a| a.as_str() != "-s" && a.as_str() != "--sandbox")
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::super::tests::test_support::sandbox_for_test;
    use super::*;

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
}
