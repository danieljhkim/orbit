//! macOS `sandbox-exec` primitive: SBPL compilation + sandboxed spawn.
//!
//! Translates a [`ResolvedFsProfile`] into a Sandbox Profile Language (SBPL)
//! payload and wraps a child process in `sandbox-exec -f <profile>`. This is
//! the OS-level enforcement seam for `backend: cli` activities.
//!
//! # Why not `--sandbox` flags on each agent CLI?
//!
//! Codex ships its own `--sandbox` flag, gemini has `-s`, claude has nothing
//! at the OS level. Building enforcement on three different CLI surfaces
//! produces three different audit stories and an asymmetric trust model.
//! Wrapping each invocation in `sandbox-exec` gives one declarative source
//! of truth — the activity's `FsProfile` — and one enforcement seam.
//!
//! # SBPL caveats
//!
//! Apple deprecated SBPL but the kernel still honors it (codex itself uses
//! it). v1 accepts that risk; the design doc records the choice. Negated
//! `!path` rules from `denyRead` / `denyModify` are emitted as explicit
//! `(deny file-read* (subpath ...))` / `(deny file-write* (subpath ...))`
//! clauses appended after the broad allows so they win in last-match-wins.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use orbit_common::types::{OrbitError, ResolvedFsProfile};
use tempfile::NamedTempFile;

/// Compile a [`ResolvedFsProfile`] into SBPL text suitable for
/// `sandbox-exec -f`.
///
/// The emitted profile:
/// - denies everything by default;
/// - allows broad reads (`file-read*`) — agent CLIs read from `/usr`,
///   `/System`, `/Library`, dyld caches, fonts, and similar locations that
///   are not realistic to enumerate;
/// - allows the syscall classes agent CLIs rely on (process, signal, mach,
///   ipc, sysctl, iokit) and unrestricted network — agents call out to
///   provider APIs;
/// - allows writes inside the resolved `modify` scope plus a small set of
///   well-known scratch areas (`/tmp`, `/private/tmp`,
///   `/private/var/folders`, `~/Library/Caches`) that tools and the
///   filesystem layer expect to write to;
/// - appends explicit `(deny ...)` clauses for any negated entry in
///   `read` / `modify` so global `denyRead` / `denyModify` rules win
///   under SBPL's last-match-wins evaluation.
///
/// Paths in `rules.modify` are emitted as-is. Callers must resolve
/// workspace-relative globs to absolute paths before invoking this
/// function — a relative `subpath` is meaningless to the kernel.
pub fn compile_macos_sandbox_profile(rules: &ResolvedFsProfile) -> Result<String, OrbitError> {
    let home = std::env::var_os("HOME");
    let codex_home = std::env::var_os("CODEX_HOME");
    let claude_config_dir = std::env::var_os("CLAUDE_CONFIG_DIR");
    compile_macos_sandbox_profile_with_env(
        rules,
        SandboxCompileEnv {
            home: home.as_deref(),
            codex_home: codex_home.as_deref(),
            claude_config_dir: claude_config_dir.as_deref(),
        },
    )
}

/// Env inputs that influence per-provider state-directory allowances in the
/// compiled SBPL profile. Threaded through a struct so tests can pin every
/// override without juggling a long parameter list.
#[derive(Default, Clone, Copy)]
struct SandboxCompileEnv<'a> {
    home: Option<&'a OsStr>,
    codex_home: Option<&'a OsStr>,
    claude_config_dir: Option<&'a OsStr>,
}

fn compile_macos_sandbox_profile_with_env(
    rules: &ResolvedFsProfile,
    env: SandboxCompileEnv<'_>,
) -> Result<String, OrbitError> {
    let SandboxCompileEnv {
        home,
        codex_home,
        claude_config_dir,
    } = env;
    let mut out = String::new();
    out.push_str("(version 1)\n");
    out.push_str("(deny default)\n");

    out.push_str("(allow file-read*)\n");
    out.push_str("(allow process*)\n");
    out.push_str("(allow signal)\n");
    out.push_str("(allow ipc-posix*)\n");
    out.push_str("(allow mach*)\n");
    out.push_str("(allow system-fsctl)\n");
    out.push_str("(allow system-socket)\n");
    // Codex's own seatbelt profile allows these provenance-related MAC
    // syscalls. Without them, macOS can fail Codex startup with a bare
    // `Operation not permitted`; revisit this if future macOS versions move
    // or rename the private Sandbox/67 operation.
    out.push_str("(allow system-mac-syscall (mac-policy-name \"vnguard\"))\n");
    out.push_str(
        "(allow system-mac-syscall (require-all (mac-policy-name \"Sandbox\") (mac-syscall-number 67)))\n",
    );
    out.push_str("(allow network*)\n");
    out.push_str("(allow sysctl*)\n");
    out.push_str("(allow iokit*)\n");

    out.push_str("(allow file-write* (subpath \"/tmp\"))\n");
    out.push_str("(allow file-write* (subpath \"/private/tmp\"))\n");
    out.push_str("(allow file-write* (subpath \"/private/var/folders\"))\n");
    out.push_str("(allow file-write* (subpath \"/dev\"))\n");
    if let Some(home) = non_empty_env_path(home) {
        let home = home.display().to_string();
        out.push_str(&format!(
            "(allow file-write* (subpath \"{}/Library/Caches\"))\n",
            sbpl_escape(&home)
        ));
        // The agent CLI inherits the sandbox into its `orbit mcp serve` child
        // (and any other `orbit ...` calls it makes). Those processes need
        // write access to the global Orbit data root so audit events, the
        // SQLite store, and run-state files can be persisted. Without this
        // the inherited child fails with `readonly database` and MCP tool
        // calls round-trip empty.
        out.push_str(&format!(
            "(allow file-write* (subpath \"{}/.orbit\"))\n",
            sbpl_escape(&home)
        ));
    }
    // Per-provider state directories. Each `backend: cli` agent CLI writes
    // setup state (sessions, settings, history, etc.) before it reads
    // Orbit's envelope. Active provider is not threaded through SBPL
    // compilation, and per-provider allowances do not widen attack surface,
    // so emit narrow allows for every supported provider's state dir
    // unconditionally.
    for state_dir in provider_state_dirs(home, codex_home, claude_config_dir) {
        out.push_str(&format!(
            "(allow file-write* (subpath \"{}\"))\n",
            sbpl_escape(&state_dir.display().to_string())
        ));
    }

    for rule in &rules.modify {
        if let Some(deny_path) = rule.strip_prefix('!') {
            out.push_str(&format!(
                "(deny file-write* {})\n",
                sbpl_filter_for_deny_rule(deny_path)
            ));
            continue;
        }
        let path = subpath_root(rule);
        out.push_str(&format!(
            "(allow file-write* (subpath \"{}\"))\n",
            sbpl_escape(&path)
        ));
    }

    for rule in &rules.read {
        if let Some(deny_path) = rule.strip_prefix('!') {
            out.push_str(&format!(
                "(deny file-read* {})\n",
                sbpl_filter_for_deny_rule(deny_path)
            ));
        }
    }

    Ok(out)
}

fn provider_state_dirs(
    home: Option<&OsStr>,
    codex_home: Option<&OsStr>,
    claude_config_dir: Option<&OsStr>,
) -> Vec<PathBuf> {
    let mut dirs = Vec::with_capacity(3);
    if let Some(dir) = codex_state_dir(home, codex_home) {
        dirs.push(dir);
    }
    if let Some(dir) = claude_state_dir(home, claude_config_dir) {
        dirs.push(dir);
    }
    if let Some(dir) = gemini_state_dir(home) {
        dirs.push(dir);
    }
    dirs
}

fn codex_state_dir(home: Option<&OsStr>, codex_home: Option<&OsStr>) -> Option<PathBuf> {
    non_empty_env_path(codex_home)
        .or_else(|| non_empty_env_path(home).map(|path| path.join(".codex")))
}

/// Claude Code documents `CLAUDE_CONFIG_DIR` as the override; otherwise the
/// CLI writes settings, sessions, projects, file-history, and todos under
/// `$HOME/.claude`.
fn claude_state_dir(home: Option<&OsStr>, claude_config_dir: Option<&OsStr>) -> Option<PathBuf> {
    non_empty_env_path(claude_config_dir)
        .or_else(|| non_empty_env_path(home).map(|path| path.join(".claude")))
}

/// Gemini CLI does not document a stable env override — it writes state
/// under `$HOME/.gemini`. If a future CLI release surfaces an override, plumb
/// it through `SandboxCompileEnv` here.
fn gemini_state_dir(home: Option<&OsStr>) -> Option<PathBuf> {
    non_empty_env_path(home).map(|path| path.join(".gemini"))
}

fn non_empty_env_path(value: Option<&OsStr>) -> Option<PathBuf> {
    let value = value?;
    if value.to_string_lossy().is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

/// Spawn `program` under `sandbox-exec -f <profile>`. Returns the running
/// [`Child`] paired with a [`NamedTempFile`] holding the compiled profile;
/// the caller must keep the `NamedTempFile` alive until the child exits, or
/// the kernel may lose the profile mid-run.
///
/// `process_group(0)` is set on Unix so the supervision layer can reap the
/// whole tree (matching the `orbit-exec::process::spawn` contract).
pub fn spawn_under_macos_sandbox(
    profile_text: &str,
    program: &str,
    args: &[String],
    env: &[(String, String)],
    stdin: Stdio,
    stdout: Stdio,
    stderr: Stdio,
) -> Result<(Child, NamedTempFile), OrbitError> {
    let mut profile_file = tempfile::Builder::new()
        .prefix("orbit-sandbox-")
        .suffix(".sb")
        .tempfile()
        .map_err(|err| {
            OrbitError::Execution(format!("failed to create sandbox profile tempfile: {err}"))
        })?;
    use std::io::Write;
    profile_file
        .write_all(profile_text.as_bytes())
        .map_err(|err| {
            OrbitError::Execution(format!("failed to write sandbox profile tempfile: {err}"))
        })?;
    profile_file
        .flush()
        .map_err(|err| OrbitError::Execution(format!("failed to flush sandbox profile: {err}")))?;

    let profile_path = profile_file.path().to_path_buf();

    let mut command = Command::new("sandbox-exec");
    command
        .arg("-f")
        .arg(&profile_path)
        .arg(program)
        .args(args)
        .envs(env.iter().map(|(key, value)| (key, value)))
        .stdin(stdin)
        .stdout(stdout)
        .stderr(stderr);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let child = command.spawn().map_err(|err| {
        OrbitError::Execution(format!(
            "failed to spawn sandbox-exec wrapper around `{program}`: {err}"
        ))
    })?;
    Ok((child, profile_file))
}

/// Returns `true` if `sandbox-exec` is on `PATH`.
pub fn sandbox_exec_available() -> bool {
    sandbox_exec_available_in(&std::env::var_os("PATH").unwrap_or_default())
}

pub(crate) fn sandbox_exec_available_in(path_var: &std::ffi::OsStr) -> bool {
    for dir in std::env::split_paths(path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let candidate = dir.join("sandbox-exec");
        if is_executable(&candidate) {
            return true;
        }
    }
    false
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_file() && (meta.permissions().mode() & 0o111) != 0,
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn sbpl_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn sbpl_filter_for_deny_rule(rule: &str) -> String {
    if deny_rule_can_use_subpath(rule) {
        let path = subpath_root(rule);
        format!("(subpath \"{}\")", sbpl_escape(&path))
    } else {
        let regex = glob_rule_to_regex(rule);
        format!("(regex \"{}\")", sbpl_escape(&regex))
    }
}

fn deny_rule_can_use_subpath(rule: &str) -> bool {
    let trimmed = rule.trim_end_matches('/');
    if !contains_glob(trimmed) {
        return true;
    }
    let Some(prefix) = trimmed.strip_suffix("/**") else {
        return false;
    };
    !contains_glob(prefix)
}

fn contains_glob(value: &str) -> bool {
    value.contains('*') || value.contains('?')
}

fn glob_rule_to_regex(rule: &str) -> String {
    let mut out = String::from("^");
    let chars: Vec<char> = rule.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' if chars.get(i + 1) == Some(&'*') => {
                if chars.get(i + 2) == Some(&'/') {
                    out.push_str("(?:.*/)?");
                    i += 3;
                } else {
                    out.push_str(".*");
                    i += 2;
                }
            }
            '*' => {
                out.push_str("[^/]*");
                i += 1;
            }
            '?' => {
                out.push_str("[^/]");
                i += 1;
            }
            c => {
                push_regex_escaped(&mut out, c);
                i += 1;
            }
        }
    }
    out.push('$');
    out
}

fn push_regex_escaped(out: &mut String, c: char) {
    if matches!(
        c,
        '.' | '+' | '(' | ')' | '|' | '^' | '$' | '{' | '}' | '[' | ']' | '\\'
    ) {
        out.push('\\');
    }
    out.push(c);
}

/// Strip glob suffixes from a rule so it can be used as a `subpath` root.
/// `subpath` matches a directory and everything beneath, so `**` wildcards
/// are redundant and `*` segments cannot be expressed in SBPL — we collapse
/// them to the longest non-glob prefix.
fn subpath_root(rule: &str) -> String {
    let trimmed = rule.trim_end_matches('/');
    let trimmed = trimmed.trim_end_matches("/**");
    if let Some(idx) = trimmed.find(['*', '?']) {
        let prefix = &trimmed[..idx];
        let prefix = prefix.trim_end_matches('/');
        if prefix.is_empty() {
            "/".to_string()
        } else {
            prefix.to_string()
        }
    } else if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_common::types::ResolvedFsProfile;

    fn profile(name: &str, read: &[&str], modify: &[&str]) -> ResolvedFsProfile {
        ResolvedFsProfile {
            name: name.to_string(),
            read: read.iter().map(|s| s.to_string()).collect(),
            modify: modify.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[derive(Default)]
    struct EnvOverrides<'a> {
        home: Option<&'a str>,
        codex_home: Option<&'a str>,
        claude_config_dir: Option<&'a str>,
    }

    fn compile_with_env(resolved: &ResolvedFsProfile, env: EnvOverrides<'_>) -> String {
        compile_macos_sandbox_profile_with_env(
            resolved,
            SandboxCompileEnv {
                home: env.home.map(OsStr::new),
                codex_home: env.codex_home.map(OsStr::new),
                claude_config_dir: env.claude_config_dir.map(OsStr::new),
            },
        )
        .expect("compile")
    }

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
    fn compile_grants_write_access_to_global_orbit_data_root() {
        // The agent CLI inherits the sandbox into `orbit mcp serve` and any
        // other `orbit ...` calls; those need to write to ~/.orbit (audit
        // events, SQLite stores, run state). Without this clause the
        // inherited child fails with `readonly database`.
        let resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo/src"]);
        let text = compile_with_env(
            &resolved,
            EnvOverrides {
                home: Some("/Users/test"),
                ..Default::default()
            },
        );
        assert!(
            text.contains("(allow file-write* (subpath \"/Users/test/.orbit\"))"),
            "missing ~/.orbit write allow: {text}"
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
            text.contains("(allow file-write* (subpath \"/Users/test/.orbit\"))"),
            "missing injected HOME/.orbit write allow: {text}"
        );
        assert_eq!(
            std::env::var_os("HOME"),
            home_before,
            "profile compilation tests must not mutate process HOME"
        );
    }

    #[test]
    fn compile_grants_write_access_to_codex_home_when_set() {
        let resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo/src"]);
        let text = compile_with_env(
            &resolved,
            EnvOverrides {
                home: Some("/Users/test"),
                codex_home: Some("/var/folders/test/codex-home"),
                ..Default::default()
            },
        );
        assert!(
            text.contains("(allow file-write* (subpath \"/var/folders/test/codex-home\"))"),
            "missing CODEX_HOME write allow: {text}"
        );
        assert!(
            !text.contains("(allow file-write* (subpath \"/Users/test/.codex\"))"),
            "CODEX_HOME should take precedence over HOME fallback: {text}"
        );
    }

    #[test]
    fn compile_grants_write_access_to_home_codex_when_codex_home_missing() {
        let resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo/src"]);
        let text = compile_with_env(
            &resolved,
            EnvOverrides {
                home: Some("/Users/test"),
                ..Default::default()
            },
        );
        assert!(
            text.contains("(allow file-write* (subpath \"/Users/test/.codex\"))"),
            "missing HOME/.codex write allow: {text}"
        );
    }

    #[test]
    fn compile_grants_write_access_to_claude_config_dir_when_set() {
        let resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo/src"]);
        let text = compile_with_env(
            &resolved,
            EnvOverrides {
                home: Some("/Users/test"),
                claude_config_dir: Some("/var/folders/test/claude-config"),
                ..Default::default()
            },
        );
        assert!(
            text.contains("(allow file-write* (subpath \"/var/folders/test/claude-config\"))"),
            "missing CLAUDE_CONFIG_DIR write allow: {text}"
        );
        assert!(
            !text.contains("(allow file-write* (subpath \"/Users/test/.claude\"))"),
            "CLAUDE_CONFIG_DIR should take precedence over HOME fallback: {text}"
        );
    }

    #[test]
    fn compile_grants_write_access_to_home_claude_when_claude_config_dir_missing() {
        let resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo/src"]);
        let text = compile_with_env(
            &resolved,
            EnvOverrides {
                home: Some("/Users/test"),
                ..Default::default()
            },
        );
        assert!(
            text.contains("(allow file-write* (subpath \"/Users/test/.claude\"))"),
            "missing HOME/.claude write allow: {text}"
        );
    }

    #[test]
    fn compile_grants_write_access_to_home_gemini() {
        let resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo/src"]);
        let text = compile_with_env(
            &resolved,
            EnvOverrides {
                home: Some("/Users/test"),
                ..Default::default()
            },
        );
        assert!(
            text.contains("(allow file-write* (subpath \"/Users/test/.gemini\"))"),
            "missing HOME/.gemini write allow: {text}"
        );
    }

    #[test]
    fn compile_emits_all_provider_state_dirs() {
        // Active provider is not threaded through SBPL compilation; emitting
        // all three keeps the profile symmetric and avoids per-provider
        // branching at compile time.
        let resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo/src"]);
        let text = compile_with_env(
            &resolved,
            EnvOverrides {
                home: Some("/Users/test"),
                ..Default::default()
            },
        );
        for dir in [".codex", ".claude", ".gemini"] {
            let needle = format!("(allow file-write* (subpath \"/Users/test/{dir}\"))");
            assert!(
                text.contains(&needle),
                "missing provider state dir allow `{needle}`: {text}"
            );
        }
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

    #[test]
    fn compile_strips_glob_suffix_for_subpath_root() {
        let resolved = profile(
            "default",
            &["/Users/test/repo"],
            &["/Users/test/repo/src/**"],
        );
        let text = compile_with_env(&resolved, EnvOverrides::default());
        assert!(
            text.contains("(allow file-write* (subpath \"/Users/test/repo/src\"))"),
            "expected glob-stripped subpath: {text}"
        );
        assert!(
            !text.contains("/src/**"),
            "subpath should not contain glob marker: {text}"
        );
    }

    #[test]
    fn compile_appends_explicit_deny_for_negated_modify_rule() {
        let mut resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo"]);
        resolved.modify.push("!/Users/test/repo/.env".to_string());
        let text = compile_with_env(&resolved, EnvOverrides::default());
        assert!(
            text.contains("(deny file-write* (subpath \"/Users/test/repo/.env\"))"),
            "missing deny clause: {text}"
        );
        let allow_pos = text
            .find("(allow file-write* (subpath \"/Users/test/repo\"))")
            .expect("allow clause present");
        let deny_pos = text
            .find("(deny file-write* (subpath \"/Users/test/repo/.env\"))")
            .expect("deny clause present");
        assert!(
            allow_pos < deny_pos,
            "deny clause must come after allow for last-match-wins: {text}"
        );
    }

    #[test]
    fn compile_uses_regex_for_non_subpath_negated_modify_glob() {
        let mut resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo"]);
        resolved
            .modify
            .push("!/Users/test/repo/**/*.env".to_string());
        let text = compile_with_env(&resolved, EnvOverrides::default());
        assert!(
            text.contains(
                "(deny file-write* (regex \"^/Users/test/repo/(?:.*/)?[^/]*\\\\.env$\"))"
            ),
            "missing regex deny for env glob: {text}"
        );
        assert!(
            !text.contains("(deny file-write* (subpath \"/Users/test/repo\"))"),
            "env glob must not collapse to a repo-wide deny: {text}"
        );
    }

    #[test]
    fn sandbox_exec_available_in_finds_executable_on_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bin = dir.path().join("sandbox-exec");
        std::fs::write(&bin, "#!/bin/sh\nexit 0\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("perms");
        }
        let path_var = std::ffi::OsString::from(dir.path().display().to_string());
        assert!(sandbox_exec_available_in(&path_var));
    }

    #[test]
    fn sandbox_exec_available_in_returns_false_when_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path_var = std::ffi::OsString::from(dir.path().display().to_string());
        assert!(!sandbox_exec_available_in(&path_var));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn compiled_profile_blocks_writes_outside_modify_scope() {
        use std::process::Command;

        if !sandbox_exec_can_apply() {
            return;
        }

        // The compiled profile broadly allows writes under /tmp,
        // /private/tmp, /private/var/folders, and ~/Library/Caches so
        // agent CLIs can use scratch space. To exercise modify-scope
        // enforcement we need a parent that lives outside all of those.
        let parent = sandbox_test_parent("modify-scope");
        let _cleanup = ScopeGuard(parent.clone());
        let dir = tempfile::Builder::new()
            .prefix("compile-")
            .tempdir_in(&parent)
            .expect("tempdir in parent");
        let allowed = dir.path().join("allowed");
        let blocked = dir.path().join("blocked");
        std::fs::create_dir_all(&allowed).expect("allowed dir");
        std::fs::create_dir_all(&blocked).expect("blocked dir");

        let resolved = ResolvedFsProfile {
            name: "default".to_string(),
            read: vec![dir.path().display().to_string()],
            modify: vec![allowed.display().to_string()],
        };
        let profile_text = compile_macos_sandbox_profile(&resolved).expect("compile sbpl");
        let mut profile_file = tempfile::Builder::new()
            .prefix("orbit-sandbox-test-")
            .suffix(".sb")
            .tempfile()
            .expect("tempfile");
        use std::io::Write;
        profile_file
            .write_all(profile_text.as_bytes())
            .expect("write profile");
        profile_file.flush().expect("flush");

        let allowed_target = allowed.join("ok");
        let allow_status = Command::new("sandbox-exec")
            .arg("-f")
            .arg(profile_file.path())
            .arg("/bin/sh")
            .arg("-c")
            .arg(format!("echo ok > {}", shell_escape(&allowed_target)))
            .status()
            .expect("run sandbox-exec");
        assert!(
            allow_status.success(),
            "expected write inside modify scope to succeed; status={allow_status:?}"
        );
        assert!(
            allowed_target.exists(),
            "allowed file was not written: {allowed_target:?}"
        );

        let blocked_target = blocked.join("nope");
        let deny_status = Command::new("sandbox-exec")
            .arg("-f")
            .arg(profile_file.path())
            .arg("/bin/sh")
            .arg("-c")
            .arg(format!("echo bad > {}", shell_escape(&blocked_target)))
            .status()
            .expect("run sandbox-exec");
        assert!(
            !deny_status.success(),
            "expected write outside modify scope to fail; status={deny_status:?}"
        );
        assert!(
            !blocked_target.exists(),
            "blocked file should not exist: {blocked_target:?}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn compiled_profile_denies_env_glob_without_blocking_other_writes() {
        use std::process::Command;

        if !sandbox_exec_can_apply() {
            return;
        }

        let parent = sandbox_test_parent("env-glob");
        let _cleanup = ScopeGuard(parent.clone());
        let dir = tempfile::Builder::new()
            .prefix("compile-env-")
            .tempdir_in(&parent)
            .expect("tempdir in parent");

        let resolved = ResolvedFsProfile {
            name: "default".to_string(),
            read: vec![dir.path().display().to_string()],
            modify: vec![
                dir.path().display().to_string(),
                format!("!{}/**/*.env", dir.path().display()),
            ],
        };
        let profile_text = compile_macos_sandbox_profile(&resolved).expect("compile sbpl");
        let mut profile_file = tempfile::Builder::new()
            .prefix("orbit-sandbox-test-")
            .suffix(".sb")
            .tempfile()
            .expect("tempfile");
        use std::io::Write;
        profile_file
            .write_all(profile_text.as_bytes())
            .expect("write profile");
        profile_file.flush().expect("flush");

        let allowed_target = dir.path().join("ok.txt");
        let allow_status = Command::new("sandbox-exec")
            .arg("-f")
            .arg(profile_file.path())
            .arg("/bin/sh")
            .arg("-c")
            .arg(format!("echo ok > {}", shell_escape(&allowed_target)))
            .status()
            .expect("run sandbox-exec");
        assert!(
            allow_status.success(),
            "env glob deny should not block non-env writes; status={allow_status:?}"
        );

        let env_target = dir.path().join("blocked.env");
        let deny_status = Command::new("sandbox-exec")
            .arg("-f")
            .arg(profile_file.path())
            .arg("/bin/sh")
            .arg("-c")
            .arg(format!("echo bad > {}", shell_escape(&env_target)))
            .status()
            .expect("run sandbox-exec");
        assert!(
            !deny_status.success(),
            "expected env glob write to fail; status={deny_status:?}"
        );
        assert!(
            !env_target.exists(),
            "env file should not exist: {env_target:?}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn compiled_profile_allows_writes_to_claude_and_gemini_state_dirs() {
        // Documented equivalent for AC #2 / #3 of T20260428-14: rather than
        // executing real provider binaries, exercise the same SBPL allow
        // clause Claude/Gemini rely on at startup. If the kernel permits a
        // write under the synthetic `.claude` / `.gemini` subpaths, the same
        // mechanism unblocks the real CLIs writing settings/sessions there.
        use std::process::Command;

        if !sandbox_exec_can_apply() {
            return;
        }

        let parent = sandbox_test_parent("provider-state");
        let _cleanup = ScopeGuard(parent.clone());
        let synthetic_home = tempfile::Builder::new()
            .prefix("synthetic-home-")
            .tempdir_in(&parent)
            .expect("synthetic home tempdir");
        let claude_dir = synthetic_home.path().join(".claude");
        let gemini_dir = synthetic_home.path().join(".gemini");
        std::fs::create_dir_all(&claude_dir).expect("claude dir");
        std::fs::create_dir_all(&gemini_dir).expect("gemini dir");

        let resolved = ResolvedFsProfile {
            name: "default".to_string(),
            read: vec![synthetic_home.path().display().to_string()],
            modify: vec![],
        };
        let profile_text = compile_macos_sandbox_profile_with_env(
            &resolved,
            SandboxCompileEnv {
                home: Some(synthetic_home.path().as_os_str()),
                codex_home: None,
                claude_config_dir: None,
            },
        )
        .expect("compile sbpl");
        let mut profile_file = tempfile::Builder::new()
            .prefix("orbit-sandbox-test-")
            .suffix(".sb")
            .tempfile()
            .expect("tempfile");
        use std::io::Write;
        profile_file
            .write_all(profile_text.as_bytes())
            .expect("write profile");
        profile_file.flush().expect("flush");

        for (label, target) in [
            ("claude", claude_dir.join("ok")),
            ("gemini", gemini_dir.join("ok")),
        ] {
            let status = Command::new("sandbox-exec")
                .arg("-f")
                .arg(profile_file.path())
                .arg("/bin/sh")
                .arg("-c")
                .arg(format!("echo ok > {}", shell_escape(&target)))
                .status()
                .expect("run sandbox-exec");
            assert!(
                status.success(),
                "expected write under synthetic ~/.{label} to succeed; status={status:?}"
            );
            assert!(
                target.exists(),
                "{label} target file was not written: {target:?}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    fn shell_escape(path: &Path) -> String {
        let s = path.display().to_string();
        format!("'{}'", s.replace('\'', "'\\''"))
    }

    #[cfg(target_os = "macos")]
    fn sandbox_exec_can_apply() -> bool {
        if !sandbox_exec_available() {
            return false;
        }

        let mut profile_file = tempfile::Builder::new()
            .prefix("orbit-sandbox-probe-")
            .suffix(".sb")
            .tempfile()
            .expect("probe profile tempfile");
        use std::io::Write;
        profile_file
            .write_all(b"(version 1)\n(allow default)\n")
            .expect("write probe profile");
        profile_file.flush().expect("flush probe profile");

        std::process::Command::new("sandbox-exec")
            .arg("-f")
            .arg(profile_file.path())
            .arg("/usr/bin/true")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[cfg(target_os = "macos")]
    static SANDBOX_TEST_PARENT_COUNTER: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

    #[cfg(target_os = "macos")]
    fn sandbox_test_parent(label: &str) -> std::path::PathBuf {
        let roots = [
            Some(std::env::current_dir().expect("current dir")),
            std::env::var_os("HOME").map(std::path::PathBuf::from),
        ];
        let suffix = SANDBOX_TEST_PARENT_COUNTER
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .to_string();
        let mut attempts = Vec::new();
        for root in roots.into_iter().flatten() {
            if is_default_write_allow_root(&root) {
                attempts.push(format!(
                    "{} is under a broad sandbox write allow",
                    root.display()
                ));
                continue;
            }
            let parent = root.join(format!(
                ".orbit-sandbox-test-{}-{label}-{suffix}",
                std::process::id()
            ));
            match std::fs::create_dir_all(&parent) {
                Ok(()) => return parent,
                Err(err) => attempts.push(format!("{}: {err}", parent.display())),
            }
        }
        panic!(
            "no writable macOS sandbox test parent outside broad write allows: {}",
            attempts.join("; ")
        );
    }

    #[cfg(target_os = "macos")]
    fn is_default_write_allow_root(path: &Path) -> bool {
        fn matches_allowed(path: &Path) -> bool {
            [
                Path::new("/tmp"),
                Path::new("/private/tmp"),
                Path::new("/private/var/folders"),
                Path::new("/dev"),
            ]
            .into_iter()
            .any(|root| path.starts_with(root))
        }

        if matches_allowed(path) {
            return true;
        }
        match path.canonicalize() {
            Ok(canonical) => matches_allowed(&canonical),
            Err(_) => false,
        }
    }

    #[cfg(target_os = "macos")]
    struct ScopeGuard(std::path::PathBuf);

    #[cfg(target_os = "macos")]
    impl Drop for ScopeGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
