use super::super::super::test_support::*;

#[test]
fn compile_emits_deny_default_and_broad_read_with_modify_subpath() {
    let resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo/src"]);
    let text = compile_with_env(&resolved, EnvOverrides::default());
    assert!(text.contains("(deny default)"));
    assert!(text.contains("(allow file-read*)"));
    assert!(
        text.contains("(allow file-write* (subpath \"/Users/test/repo/src\"))"),
        "missing modify subpath clause: {text}"
    );
}

#[test]
fn compile_grants_write_access_to_global_orbit_log_dir() {
    // The agent CLI inherits the sandbox into `orbit mcp serve` and any
    // other `orbit ...` calls. The JSONL tracing layer resolves its
    // HOME-based path before runtime root resolution, so only the log
    // directory is granted here; store and artifact roots are appended by
    // the runtime sandbox resolver.
    let resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo/src"]);
    let text = compile_with_env(
        &resolved,
        EnvOverrides {
            home: Some("/Users/test"),
            ..Default::default()
        },
    );
    assert!(
        text.contains("(allow file-write* (subpath \"/Users/test/.orbit/state/logs\"))"),
        "missing ~/.orbit/state/logs write allow: {text}"
    );
    assert!(
        !text.contains("(allow file-write* (subpath \"/Users/test/.orbit\"))"),
        "profile must not broadly allow HOME/.orbit writes: {text}"
    );
}

#[test]
fn compile_with_env_does_not_mutate_process_home() {
    let home_before = std::env::var_os("HOME");
    let resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo/src"]);
    let text = compile_with_env(
        &resolved,
        EnvOverrides {
            home: Some("/Users/test"),
            ..Default::default()
        },
    );
    assert!(
        text.contains("(allow file-write* (subpath \"/Users/test/.orbit/state/logs\"))"),
        "missing injected HOME/.orbit/state/logs write allow: {text}"
    );
    assert_eq!(
        std::env::var_os("HOME"),
        home_before,
        "profile compilation tests must not mutate process HOME"
    );
}

#[test]
fn compile_allows_macos_sandbox_provenance_syscall() {
    let resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo/src"]);
    let text = compile_with_env(
        &resolved,
        EnvOverrides {
            home: Some("/Users/test"),
            ..Default::default()
        },
    );
    assert!(
        text.contains("(allow system-mac-syscall (mac-policy-name \"vnguard\"))"),
        "missing vnguard mac syscall allow: {text}"
    );
    assert!(
        text.contains(
            "(allow system-mac-syscall (require-all (mac-policy-name \"Sandbox\") (mac-syscall-number 67)))"
        ),
        "missing Sandbox mac syscall allow: {text}"
    );
}
