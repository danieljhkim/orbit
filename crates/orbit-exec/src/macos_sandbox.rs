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

const TRUSTED_SANDBOX_EXEC_PATHS: &[&str] = &["/usr/bin/sandbox-exec"];

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
///   `/private/var/folders`, `~/Library/Caches`, and the HOME-derived Orbit
///   JSONL log directory) that tools and the filesystem layer expect to write to;
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
    let grok_home = std::env::var_os("GROK_HOME");
    compile_macos_sandbox_profile_with_env(
        rules,
        SandboxCompileEnv {
            home: home.as_deref(),
            codex_home: codex_home.as_deref(),
            claude_config_dir: claude_config_dir.as_deref(),
            grok_home: grok_home.as_deref(),
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
    grok_home: Option<&'a OsStr>,
}

fn compile_macos_sandbox_profile_with_env(
    rules: &ResolvedFsProfile,
    env: SandboxCompileEnv<'_>,
) -> Result<String, OrbitError> {
    let SandboxCompileEnv {
        home,
        codex_home,
        claude_config_dir,
        grok_home,
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
        // (and any other `orbit ...` calls it makes). Logging initializes
        // before the child can resolve Orbit's runtime roots, so the profile
        // carries the one HOME-derived path that must be writable up front.
        // Runtime-specific store/artifact paths are appended by orbit-core's
        // sandbox resolver instead of granting the whole HOME/.orbit tree.
        out.push_str(&format!(
            "(allow file-write* (subpath \"{}/.orbit/state/logs\"))\n",
            sbpl_escape(&home)
        ));
    }
    // Per-provider state directories. Each `backend: cli` agent CLI writes
    // setup state (sessions, settings, history, etc.) before it reads
    // Orbit's envelope. Active provider is not threaded through SBPL
    // compilation, and per-provider allowances do not widen attack surface,
    // so emit narrow allows for every supported provider's state dir
    // unconditionally.
    for state_dir in provider_state_dirs(home, codex_home, claude_config_dir, grok_home) {
        out.push_str(&format!(
            "(allow file-write* (subpath \"{}\"))\n",
            sbpl_escape(&state_dir.display().to_string())
        ));
    }
    emit_claude_home_json_allows(home, claude_config_dir, &mut out);
    emit_grok_state_file_allows(home, grok_home, &mut out);

    for rule in &rules.modify {
        if let Some(deny_path) = rule.strip_prefix('!') {
            out.push_str(&format!(
                "(deny file-write* {})\n",
                sbpl_filter_for_deny_rule(deny_path)
            ));
            continue;
        }
        out.push_str(&format!(
            "(allow file-write* {})\n",
            sbpl_filter_for_allow_rule(rule)
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
    grok_home: Option<&OsStr>,
) -> Vec<PathBuf> {
    let mut dirs = Vec::with_capacity(4);
    if let Some(dir) = codex_state_dir(home, codex_home) {
        dirs.push(dir);
    }
    if let Some(dir) = claude_state_dir(home, claude_config_dir) {
        dirs.push(dir);
    }
    if let Some(dir) = gemini_state_dir(home) {
        dirs.push(dir);
    }
    if let Some(dir) = grok_state_dir(home, grok_home) {
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

/// Process-env wrapper around [`claude_state_dir`]. Returns the writable
/// state directory Claude Code uses at runtime — `$CLAUDE_CONFIG_DIR` if
/// set, otherwise `$HOME/.claude`. Returns `None` only when both env vars
/// are unset or empty. Callers in `backend: cli` use this to land
/// auxiliary CLI outputs (e.g. `--debug-file`) at a sandbox-allowed path
/// instead of the workspace, where `denyModify: .orbit/**` would block
/// startup-time writes.
pub fn claude_state_dir_from_env() -> Option<PathBuf> {
    let claude_config_dir = std::env::var_os("CLAUDE_CONFIG_DIR");
    let home = std::env::var_os("HOME");
    claude_state_dir(home.as_deref(), claude_config_dir.as_deref())
}

/// Gemini CLI does not document a stable env override — it writes state
/// under `$HOME/.gemini`. If a future CLI release surfaces an override, plumb
/// it through `SandboxCompileEnv` here.
fn gemini_state_dir(home: Option<&OsStr>) -> Option<PathBuf> {
    non_empty_env_path(home).map(|path| path.join(".gemini"))
}

/// Grok Build documents `GROK_HOME` as the override for its config/state
/// directory; otherwise it writes under `$HOME/.grok`.
fn grok_state_dir(home: Option<&OsStr>, grok_home: Option<&OsStr>) -> Option<PathBuf> {
    non_empty_env_path(grok_home)
        .or_else(|| non_empty_env_path(home).map(|path| path.join(".grok")))
}

/// Process-env wrapper around [`grok_state_dir`]. Returns the writable state
/// directory Grok Build uses at runtime — `$GROK_HOME` if set, otherwise
/// `$HOME/.grok`. Returns `None` only when both env vars are unset or empty.
pub fn grok_state_dir_from_env() -> Option<PathBuf> {
    let home = std::env::var_os("HOME");
    let grok_home = std::env::var_os("GROK_HOME");
    grok_state_dir(home.as_deref(), grok_home.as_deref())
}

fn non_empty_env_path(value: Option<&OsStr>) -> Option<PathBuf> {
    let value = value?;
    if value.to_string_lossy().is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

/// Claude Code persists its main settings to `$HOME/.claude.json`, a sibling
/// *file* (with `.lock` and atomic-write `.tmp.<pid>.<ms_ts>` siblings) of
/// the `$HOME/.claude/` directory. SBPL `subpath` does not match these
/// siblings, so the per-provider state-dir clause emitted by
/// `provider_state_dirs` is not enough — Claude under sandbox would hang
/// waiting on its own lockfile.
///
/// Skip when `CLAUDE_CONFIG_DIR` is set: with the override, Claude writes
/// `<override>/.claude.json` (and the lock/tmp siblings) inside the override
/// directory, already covered by the existing subpath clause.
fn emit_claude_home_json_allows(
    home: Option<&OsStr>,
    claude_config_dir: Option<&OsStr>,
    out: &mut String,
) {
    if non_empty_env_path(claude_config_dir).is_some() {
        return;
    }
    let Some(home) = non_empty_env_path(home) else {
        return;
    };
    let home_str = home.display().to_string();
    out.push_str(&format!(
        "(allow file-write* (literal \"{}/.claude.json\"))\n",
        sbpl_escape(&home_str)
    ));
    out.push_str(&format!(
        "(allow file-write* (literal \"{}/.claude.json.lock\"))\n",
        sbpl_escape(&home_str)
    ));
    let mut tmp_regex = String::from("^");
    for c in home_str.chars() {
        push_regex_escaped(&mut tmp_regex, c);
    }
    tmp_regex.push_str("/\\.claude\\.json\\.tmp\\.[0-9]+\\.[0-9]+$");
    out.push_str(&format!(
        "(allow file-write* (regex \"{}\"))\n",
        sbpl_escape(&tmp_regex)
    ));
}

/// Grok keeps its JSON state and companion lock/tmp files under `GROK_HOME`
/// (default `$HOME/.grok`). The state-dir `subpath` allow already covers
/// these, but emitting explicit rules mirrors Claude's lockfile treatment and
/// keeps the startup-critical files visible in compiled profiles.
fn emit_grok_state_file_allows(home: Option<&OsStr>, grok_home: Option<&OsStr>, out: &mut String) {
    let Some(state_dir) = grok_state_dir(home, grok_home) else {
        return;
    };

    for file_name in ["auth.json", "mcp_credentials.json", "models_cache.json"] {
        emit_grok_json_file_allow(&state_dir, file_name, out);
    }

    let state_dir_str = state_dir.display().to_string();
    let mut lock_regex = String::from("^");
    push_regex_escaped_str(&mut lock_regex, &state_dir_str);
    lock_regex.push_str("/mcp_auth_[^/]+\\.lock$");
    out.push_str(&format!(
        "(allow file-write* (regex \"{}\"))\n",
        sbpl_escape(&lock_regex)
    ));
}

fn emit_grok_json_file_allow(state_dir: &Path, file_name: &str, out: &mut String) {
    let path = state_dir.join(file_name);
    let path_str = path.display().to_string();
    out.push_str(&format!(
        "(allow file-write* (literal \"{}\"))\n",
        sbpl_escape(&path_str)
    ));
    out.push_str(&format!(
        "(allow file-write* (literal \"{}.lock\"))\n",
        sbpl_escape(&path_str)
    ));

    let mut tmp_regex = String::from("^");
    push_regex_escaped_str(&mut tmp_regex, &path_str);
    tmp_regex.push_str("\\.tmp(?:\\.[0-9]+)*$");
    out.push_str(&format!(
        "(allow file-write* (regex \"{}\"))\n",
        sbpl_escape(&tmp_regex)
    ));
}

/// Spawn `program` under `sandbox-exec -f <profile>`. Returns the running
/// [`Child`] paired with a [`NamedTempFile`] holding the compiled profile;
/// the caller must keep the `NamedTempFile` alive until the child exits, or
/// the kernel may lose the profile mid-run.
/// When `cwd` is set, it is applied to the outer `sandbox-exec` wrapper and
/// inherited by the wrapped child.
///
/// `process_group(0)` is set on Unix so the supervision layer can reap the
/// whole tree (matching the `orbit-exec::process::spawn` contract).
pub struct MacosSandboxSpawnRequest<'a> {
    pub profile_text: &'a str,
    pub program: &'a str,
    pub args: &'a [String],
    pub env: &'a [(String, String)],
    pub cwd: Option<&'a Path>,
    pub stdin: Stdio,
    pub stdout: Stdio,
    pub stderr: Stdio,
}

pub fn spawn_under_macos_sandbox(
    request: MacosSandboxSpawnRequest<'_>,
) -> Result<(Child, NamedTempFile), OrbitError> {
    let MacosSandboxSpawnRequest {
        profile_text,
        program,
        args,
        env,
        cwd,
        stdin,
        stdout,
        stderr,
    } = request;

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

    let sandbox_exec_path = sandbox_exec_path_or_error()?;
    let mut command = Command::new(&sandbox_exec_path);
    command
        .arg("-f")
        .arg(&profile_path)
        .arg(program)
        .args(args)
        .envs(env.iter().map(|(key, value)| (key, value)))
        .stdin(stdin)
        .stdout(stdout)
        .stderr(stderr);
    if let Some(path) = cwd {
        command.current_dir(path);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let child = command.spawn().map_err(|err| {
        OrbitError::Execution(format!(
            "failed to spawn trusted sandbox-exec `{}` around `{program}`: {err}",
            sandbox_exec_path.display()
        ))
    })?;
    Ok((child, profile_file))
}

/// Returns the stable program path used in audit logs for sandboxed CLI
/// invocations. The real spawn path is resolved again at execution time so
/// missing binaries still fail closed.
pub fn sandbox_exec_program_for_audit() -> &'static str {
    TRUSTED_SANDBOX_EXEC_PATHS[0]
}

/// Returns `true` if a trusted absolute `sandbox-exec` binary is available.
pub fn sandbox_exec_available() -> bool {
    sandbox_exec_path().is_some()
}

/// Human-facing reason used when fail-closed sandboxing cannot find the
/// trusted wrapper.
pub fn sandbox_exec_unavailable_message() -> String {
    format!(
        "trusted sandbox-exec not available at {}",
        TRUSTED_SANDBOX_EXEC_PATHS.join(", ")
    )
}

/// Resolve `sandbox-exec` from trusted absolute locations only.
pub fn sandbox_exec_path() -> Option<PathBuf> {
    sandbox_exec_path_from(TRUSTED_SANDBOX_EXEC_PATHS.iter().map(Path::new))
}

fn sandbox_exec_path_or_error() -> Result<PathBuf, OrbitError> {
    sandbox_exec_path().ok_or_else(|| OrbitError::Execution(sandbox_exec_unavailable_message()))
}

fn sandbox_exec_path_from<I, P>(candidates: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    candidates
        .into_iter()
        .map(|candidate| candidate.as_ref().to_path_buf())
        .find(|candidate| candidate.is_absolute() && is_executable(candidate))
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
    if rule_can_use_subpath(rule) {
        let path = subpath_root(rule);
        format!("(subpath \"{}\")", sbpl_escape(&path))
    } else {
        let regex = glob_rule_to_regex(rule);
        format!("(regex \"{}\")", sbpl_escape(&regex))
    }
}

fn sbpl_filter_for_allow_rule(rule: &str) -> String {
    if rule_can_use_subpath(rule) {
        let path = subpath_root(rule);
        format!("(subpath \"{}\")", sbpl_escape(&path))
    } else {
        let regex = glob_rule_to_regex(rule);
        format!("(regex \"{}\")", sbpl_escape(&regex))
    }
}

fn rule_can_use_subpath(rule: &str) -> bool {
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

fn push_regex_escaped_str(out: &mut String, value: &str) {
    for c in value.chars() {
        push_regex_escaped(out, c);
    }
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
        grok_home: Option<&'a str>,
    }

    fn compile_with_env(resolved: &ResolvedFsProfile, env: EnvOverrides<'_>) -> String {
        compile_macos_sandbox_profile_with_env(
            resolved,
            SandboxCompileEnv {
                home: env.home.map(OsStr::new),
                codex_home: env.codex_home.map(OsStr::new),
                claude_config_dir: env.claude_config_dir.map(OsStr::new),
                grok_home: env.grok_home.map(OsStr::new),
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
    fn compile_grants_write_access_to_home_claude_json_when_claude_config_dir_missing() {
        let resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo/src"]);
        let text = compile_with_env(
            &resolved,
            EnvOverrides {
                home: Some("/Users/test"),
                ..Default::default()
            },
        );
        assert!(
            text.contains("(allow file-write* (literal \"/Users/test/.claude.json\"))"),
            "missing HOME/.claude.json write allow: {text}"
        );
        assert!(
            text.contains("(allow file-write* (literal \"/Users/test/.claude.json.lock\"))"),
            "missing HOME/.claude.json.lock write allow: {text}"
        );
        assert!(
            text.contains(
                "(allow file-write* (regex \"^/Users/test/\\\\.claude\\\\.json\\\\.tmp\\\\.[0-9]+\\\\.[0-9]+$\"))"
            ),
            "missing HOME/.claude.json.tmp.<pid>.<ts> regex allow: {text}"
        );
    }

    #[test]
    fn compile_does_not_emit_home_claude_json_allow_when_claude_config_dir_set() {
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
            !text.contains("/Users/test/.claude.json"),
            "HOME/.claude.json sibling allow must be skipped when CLAUDE_CONFIG_DIR is set: {text}"
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
    fn grok_state_dir_prefers_grok_home_override() {
        assert_eq!(
            grok_state_dir(
                Some(OsStr::new("/Users/test")),
                Some(OsStr::new("/tmp/grok-home"))
            ),
            Some(PathBuf::from("/tmp/grok-home"))
        );
    }

    #[test]
    fn grok_state_dir_falls_back_to_home_dot_grok() {
        assert_eq!(
            grok_state_dir(Some(OsStr::new("/Users/test")), None),
            Some(PathBuf::from("/Users/test/.grok"))
        );
    }

    #[test]
    fn grok_state_dir_from_env_reads_runtime_env() {
        const EXPECTED_ENV: &str = "ORBIT_TEST_EXPECTED_GROK_STATE_DIR";
        if let Some(expected) = std::env::var_os(EXPECTED_ENV) {
            if expected == OsStr::new("__none__") {
                assert_eq!(grok_state_dir_from_env(), None);
            } else {
                assert_eq!(
                    grok_state_dir_from_env(),
                    Some(PathBuf::from(expected)),
                    "GROK_HOME should take precedence over HOME"
                );
            }
            return;
        }

        fn run_case(expected: &str, grok_home: Option<&str>, home: Option<&str>) {
            let mut command = std::process::Command::new(
                std::env::current_exe().expect("current test executable"),
            );
            command
                .arg("grok_state_dir_from_env_reads_runtime_env")
                .arg("--exact")
                .arg("--nocapture")
                .arg("--test-threads=1")
                .env(EXPECTED_ENV, expected);
            match grok_home {
                Some(value) => {
                    command.env("GROK_HOME", value);
                }
                None => {
                    command.env_remove("GROK_HOME");
                }
            }
            match home {
                Some(value) => {
                    command.env("HOME", value);
                }
                None => {
                    command.env_remove("HOME");
                }
            }
            let status = command.status().expect("run env helper child test");
            assert!(status.success(), "child env helper case failed: {status:?}");
        }

        run_case("/tmp/grok-home", Some("/tmp/grok-home"), Some("/tmp/home"));
        run_case("/tmp/home/.grok", None, Some("/tmp/home"));
        run_case("__none__", None, None);
    }

    #[test]
    fn compile_grants_write_access_to_grok_home_when_set() {
        let resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo/src"]);
        let text = compile_with_env(
            &resolved,
            EnvOverrides {
                home: Some("/Users/test"),
                grok_home: Some("/var/folders/test/grok-home"),
                ..Default::default()
            },
        );
        assert!(
            text.contains("(allow file-write* (subpath \"/var/folders/test/grok-home\"))"),
            "missing GROK_HOME write allow: {text}"
        );
        assert!(
            !text.contains("(allow file-write* (subpath \"/Users/test/.grok\"))"),
            "GROK_HOME should take precedence over HOME fallback: {text}"
        );
    }

    #[test]
    fn compile_grants_write_access_to_home_grok_when_grok_home_missing() {
        let resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo/src"]);
        let text = compile_with_env(
            &resolved,
            EnvOverrides {
                home: Some("/Users/test"),
                ..Default::default()
            },
        );
        assert!(
            text.contains("(allow file-write* (subpath \"/Users/test/.grok\"))"),
            "missing HOME/.grok write allow: {text}"
        );
    }

    #[test]
    fn compile_emits_explicit_grok_json_lock_and_tmp_allows() {
        let resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo/src"]);
        let text = compile_with_env(
            &resolved,
            EnvOverrides {
                home: Some("/Users/test"),
                ..Default::default()
            },
        );
        assert!(
            text.contains("(allow file-write* (literal \"/Users/test/.grok/auth.json\"))"),
            "missing Grok auth.json write allow: {text}"
        );
        assert!(
            text.contains("(allow file-write* (literal \"/Users/test/.grok/auth.json.lock\"))"),
            "missing Grok auth.json.lock write allow: {text}"
        );
        assert!(
            text.contains(
                "(allow file-write* (regex \"^/Users/test/\\\\.grok/auth\\\\.json\\\\.tmp(?:\\\\.[0-9]+)*$\"))"
            ),
            "missing Grok auth.json tmp regex allow: {text}"
        );
        assert!(
            text.contains(
                "(allow file-write* (literal \"/Users/test/.grok/mcp_credentials.json\"))"
            ),
            "missing Grok MCP credentials JSON write allow: {text}"
        );
        assert!(
            text.contains(
                "(allow file-write* (regex \"^/Users/test/\\\\.grok/mcp_auth_[^/]+\\\\.lock$\"))"
            ),
            "missing Grok MCP OAuth lock regex allow: {text}"
        );
    }

    #[test]
    fn compile_emits_all_provider_state_dirs() {
        // Active provider is not threaded through SBPL compilation; emitting
        // every supported provider keeps the profile symmetric and avoids per-provider
        // branching at compile time.
        let resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo/src"]);
        let text = compile_with_env(
            &resolved,
            EnvOverrides {
                home: Some("/Users/test"),
                ..Default::default()
            },
        );
        for dir in [".codex", ".claude", ".gemini", ".grok"] {
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
    fn compile_uses_regex_for_non_subpath_positive_modify_glob() {
        let resolved = profile(
            "default",
            &["/Users/test/repo"],
            &["/Users/test/.orbit/orbit.db*"],
        );
        let text = compile_with_env(&resolved, EnvOverrides::default());
        assert!(
            text.contains(
                "(allow file-write* (regex \"^/Users/test/\\\\.orbit/orbit\\\\.db[^/]*$\"))"
            ),
            "missing regex allow for SQLite sidecar glob: {text}"
        );
        assert!(
            !text.contains("(allow file-write* (subpath \"/Users/test/.orbit\"))"),
            "positive file glob must not collapse to the whole Orbit root: {text}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn compiled_profile_allows_nested_orbit_runtime_writes_without_home_orbit_reallow() {
        use std::process::Command;

        if !sandbox_exec_can_apply() {
            return;
        }

        let parent = sandbox_test_parent("orbit-runtime-roots");
        let _cleanup = ScopeGuard(parent.clone());
        let home = parent.join("home");
        let global = home.join(".orbit");
        let workspace = parent.join("repo/.orbit");
        std::fs::create_dir_all(global.join("state/logs")).expect("global log dir");
        std::fs::create_dir_all(global.join("tasks")).expect("global tasks dir");
        std::fs::create_dir_all(workspace.join("state")).expect("workspace state dir");

        let log_path = global.join("state/logs/orbit.jsonl");
        let db_wal_path = global.join("orbit.db-wal");
        let artifact_path = global
            .join("tasks/workspaces/orbit-test/ORB-00009/artifacts/files/planning-duel")
            .join("planner_a.md");
        let semantic_wal_path = workspace.join("state/semantic.db-wal");
        let denied_path = global.join("not-allowed.txt");

        let resolved = ResolvedFsProfile {
            name: "gemini-direct-agent".to_string(),
            read: vec![parent.display().to_string()],
            modify: vec![
                format!("{}/state/logs/**", global.display()),
                format!("{}/orbit.db*", global.display()),
                format!("{}/tasks/**", global.display()),
                format!("{}/state/semantic.db*", workspace.display()),
            ],
        };
        let home_str = home.to_string_lossy().into_owned();
        let profile_text = compile_with_env(
            &resolved,
            EnvOverrides {
                home: Some(&home_str),
                ..Default::default()
            },
        );
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

        let script = format!(
            "set -e\n: > {}\n: > {}\nmkdir -p {}\nprintf '%s\\n' '*authored by: gemini / gemini-3.1-pro*' > {}\n: > {}\nif : > {} 2>/dev/null; then exit 99; else exit 0; fi\n",
            shell_escape(&log_path),
            shell_escape(&db_wal_path),
            shell_escape(artifact_path.parent().expect("artifact parent")),
            shell_escape(&artifact_path),
            shell_escape(&semantic_wal_path),
            shell_escape(&denied_path),
        );
        let status = Command::new(sandbox_exec_path_for_test())
            .arg("-f")
            .arg(profile_file.path())
            .arg("/bin/sh")
            .arg("-c")
            .arg(script)
            .env("HOME", &home)
            .status()
            .expect("run sandbox-exec");

        assert!(
            status.success(),
            "expected Orbit runtime writes to succeed while arbitrary HOME/.orbit write is denied; status={status:?}"
        );
        assert!(log_path.exists(), "log file should be writable");
        assert!(db_wal_path.exists(), "SQLite sidecar should be writable");
        assert!(
            artifact_path.exists(),
            "planner artifact should be writable"
        );
        assert!(
            semantic_wal_path.exists(),
            "semantic sidecar should be writable"
        );
        assert!(
            !denied_path.exists(),
            "arbitrary HOME/.orbit write should remain denied"
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
    fn compile_emits_explicit_read_deny_for_negated_read_rule() {
        // Invariant: `denyRead` rules (negated entries in `read`) must
        // translate to explicit `(deny file-read* ...)` clauses appended
        // after the broad `(allow file-read*)` so they win under
        // last-match-wins. This is the kernel-side complement to
        // `compile_appends_explicit_deny_for_negated_modify_rule`.
        let mut resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo"]);
        resolved.read.push("!/Users/test/repo/.env".to_string());
        let text = compile_with_env(&resolved, EnvOverrides::default());
        assert!(
            text.contains("(deny file-read* (subpath \"/Users/test/repo/.env\"))"),
            "missing deny file-read* clause: {text}"
        );
        let allow_pos = text.find("(allow file-read*)").expect("broad read allow");
        let deny_pos = text
            .find("(deny file-read* (subpath \"/Users/test/repo/.env\"))")
            .expect("read deny clause");
        assert!(
            allow_pos < deny_pos,
            "deny file-read* must come after broad allow for last-match-wins: {text}"
        );
    }

    #[test]
    fn compile_uses_regex_for_non_subpath_negated_read_glob() {
        // Invariant: a `denyRead` rule with a non-trivial glob (e.g.
        // `!**/secrets/**`) must compile to a regex deny clause, not a
        // collapsed subpath that would over-match.
        let mut resolved = profile("default", &["/Users/test/repo"], &["/Users/test/repo"]);
        resolved.read.push("!/Users/test/repo/**/*.env".to_string());
        let text = compile_with_env(&resolved, EnvOverrides::default());
        assert!(
            text.contains("(deny file-read* (regex \"^/Users/test/repo/(?:.*/)?[^/]*\\\\.env$\"))"),
            "missing regex read deny: {text}"
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
    fn sandbox_exec_path_from_uses_trusted_absolute_candidate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bin = dir.path().join("sandbox-exec");
        std::fs::write(&bin, "#!/bin/sh\nexit 0\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("perms");
        }
        assert_eq!(sandbox_exec_path_from([bin.as_path()]), Some(bin));
    }

    #[test]
    fn sandbox_exec_path_from_rejects_relative_candidates() {
        let bin = Path::new("sandbox-exec");
        assert_eq!(sandbox_exec_path_from([bin]), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn spawn_under_macos_sandbox_ignores_fake_sandbox_exec_on_path() {
        if !sandbox_exec_can_apply() {
            return;
        }

        let temp = tempfile::tempdir().expect("tempdir");
        let fake_dir = temp.path().join("fake-bin");
        std::fs::create_dir_all(&fake_dir).expect("fake dir");
        let marker = temp.path().join("fake-used");
        let fake = fake_dir.join("sandbox-exec");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\necho fake > {}\nexit 77\n",
                shell_escape(&marker)
            ),
        )
        .expect("write fake sandbox-exec");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))
                .expect("fake perms");
        }

        let poisoned_path = format!("{}:/usr/bin:/bin", fake_dir.display());
        let args = ["-c".to_string(), "exit 0".to_string()];
        let env = [("PATH".to_string(), poisoned_path)];
        let (child, _profile_file) = spawn_under_macos_sandbox(MacosSandboxSpawnRequest {
            profile_text: "(version 1)\n(allow default)\n",
            program: "/bin/sh",
            args: &args,
            env: &env,
            cwd: None,
            stdin: Stdio::null(),
            stdout: Stdio::piped(),
            stderr: Stdio::piped(),
        })
        .expect("spawn sandboxed child");
        let output = child.wait_with_output().expect("wait for child");

        assert!(
            output.status.success(),
            "trusted sandbox-exec should run child despite fake PATH entry; stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !marker.exists(),
            "fake sandbox-exec on PATH should not have been executed"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn spawn_under_macos_sandbox_runs_program_in_provided_cwd() {
        if !sandbox_exec_can_apply() {
            return;
        }

        let parent = sandbox_test_parent("cwd");
        let _cleanup = ScopeGuard(parent.clone());
        let dir = tempfile::Builder::new()
            .prefix("sandbox-cwd-")
            .tempdir_in(&parent)
            .expect("cwd tempdir");
        let cwd = dir.path().canonicalize().expect("canonical cwd");
        let args = ["-c".to_string(), "pwd".to_string()];
        let (child, _profile_file) = spawn_under_macos_sandbox(MacosSandboxSpawnRequest {
            profile_text: "(version 1)\n(allow default)\n",
            program: "/bin/sh",
            args: &args,
            env: &[],
            cwd: Some(&cwd),
            stdin: Stdio::null(),
            stdout: Stdio::piped(),
            stderr: Stdio::piped(),
        })
        .expect("spawn sandboxed child");
        let output = child.wait_with_output().expect("wait for child");

        assert!(
            output.status.success(),
            "sandboxed pwd should succeed; stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("stdout utf8"),
            format!("{}\n", cwd.display())
        );
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
        let allow_status = Command::new(sandbox_exec_path_for_test())
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
        let deny_status = Command::new(sandbox_exec_path_for_test())
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
    fn compiled_profile_denies_reads_to_negated_read_path() {
        // Invariant: an SBPL profile compiled from `read: [base, !secrets]`
        // must let the kernel block reads of `secrets/...` while still
        // allowing reads of sibling paths under `base`. This is the
        // runtime complement to `compile_emits_explicit_read_deny_for_negated_read_rule`.
        use std::process::Command;

        if !sandbox_exec_can_apply() {
            return;
        }

        let parent = sandbox_test_parent("read-deny");
        let _cleanup = ScopeGuard(parent.clone());
        let dir = tempfile::Builder::new()
            .prefix("compile-readdeny-")
            .tempdir_in(&parent)
            .expect("tempdir in parent");
        let secrets_dir = dir.path().join("secrets");
        std::fs::create_dir_all(&secrets_dir).expect("secrets dir");
        let secret_path = secrets_dir.join("api.key");
        std::fs::write(&secret_path, b"top-secret").expect("write secret");
        let public_path = dir.path().join("public.txt");
        std::fs::write(&public_path, b"public-data").expect("write public");

        let resolved = ResolvedFsProfile {
            name: "default".to_string(),
            read: vec![
                dir.path().display().to_string(),
                format!("!{}", secrets_dir.display()),
            ],
            modify: vec![],
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

        // Allowed read of public_path succeeds.
        let allow_status = Command::new(sandbox_exec_path_for_test())
            .arg("-f")
            .arg(profile_file.path())
            .arg("/bin/sh")
            .arg("-c")
            .arg(format!("cat {}", shell_escape(&public_path)))
            .status()
            .expect("run sandbox-exec");
        assert!(
            allow_status.success(),
            "public read should be allowed; status={allow_status:?}"
        );

        // Denied read of secret_path fails.
        let deny_status = Command::new(sandbox_exec_path_for_test())
            .arg("-f")
            .arg(profile_file.path())
            .arg("/bin/sh")
            .arg("-c")
            .arg(format!("cat {}", shell_escape(&secret_path)))
            .status()
            .expect("run sandbox-exec");
        assert!(
            !deny_status.success(),
            "secrets read should be denied by negated read rule; status={deny_status:?}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn compiled_profile_for_realistic_agent_loop_profile_allows_repo_writes_denies_dotenv() {
        // Realistic activity profile boundary test (AC #2). Synthesize an
        // `agent_loop`-style profile: read=[repo], modify=[repo, !repo/.env].
        // Exercise allow + deny in one process: writing `repo/src/foo.rs`
        // succeeds; writing `repo/.env` fails. Mirrors how an `agent_loop`
        // step would be sandboxed at runtime.
        use std::process::Command;

        if !sandbox_exec_can_apply() {
            return;
        }

        let parent = sandbox_test_parent("agent-loop-realistic");
        let _cleanup = ScopeGuard(parent.clone());
        let repo = tempfile::Builder::new()
            .prefix("agent-loop-")
            .tempdir_in(&parent)
            .expect("repo tempdir");
        let src_dir = repo.path().join("src");
        std::fs::create_dir_all(&src_dir).expect("src dir");

        let resolved = ResolvedFsProfile {
            name: "agent_loop".to_string(),
            read: vec![repo.path().display().to_string()],
            modify: vec![
                repo.path().display().to_string(),
                format!("!{}/.env", repo.path().display()),
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

        let source_target = src_dir.join("foo.rs");
        let env_target = repo.path().join(".env");

        let source_status = Command::new(sandbox_exec_path_for_test())
            .arg("-f")
            .arg(profile_file.path())
            .arg("/bin/sh")
            .arg("-c")
            .arg(format!(
                "echo 'fn main() {{}}' > {}",
                shell_escape(&source_target)
            ))
            .status()
            .expect("run sandbox-exec");
        assert!(
            source_status.success(),
            "agent_loop must be able to write source files; status={source_status:?}"
        );
        assert!(source_target.exists(), "source file not written");

        let env_status = Command::new(sandbox_exec_path_for_test())
            .arg("-f")
            .arg(profile_file.path())
            .arg("/bin/sh")
            .arg("-c")
            .arg(format!("echo 'KEY=secret' > {}", shell_escape(&env_target)))
            .status()
            .expect("run sandbox-exec");
        assert!(
            !env_status.success(),
            "agent_loop must be blocked from writing .env; status={env_status:?}"
        );
        assert!(!env_target.exists(), ".env should not have been written");
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
        let allow_status = Command::new(sandbox_exec_path_for_test())
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
        let deny_status = Command::new(sandbox_exec_path_for_test())
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
    fn compiled_profile_allows_writes_to_provider_state_dirs() {
        // Documented equivalent for AC #2 / #3 of T20260428-14: rather than
        // executing real provider binaries, exercise the same SBPL allow
        // clause provider CLIs rely on at startup. If the kernel permits a
        // write under the synthetic provider state subpaths, the same
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
        let grok_dir = synthetic_home.path().join(".grok");
        std::fs::create_dir_all(&claude_dir).expect("claude dir");
        std::fs::create_dir_all(&gemini_dir).expect("gemini dir");
        std::fs::create_dir_all(&grok_dir).expect("grok dir");

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
                grok_home: None,
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
            ("grok", grok_dir.join("ok")),
        ] {
            let status = Command::new(sandbox_exec_path_for_test())
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
    #[test]
    fn compiled_profile_allows_writes_to_grok_json_lock_and_tmp_files() {
        use std::process::Command;

        if !sandbox_exec_can_apply() {
            return;
        }

        let parent = sandbox_test_parent("grok-json-locks");
        let _cleanup = ScopeGuard(parent.clone());
        let synthetic_home = tempfile::Builder::new()
            .prefix("synthetic-home-")
            .tempdir_in(&parent)
            .expect("synthetic home tempdir");
        let grok_dir = synthetic_home.path().join(".grok");
        std::fs::create_dir_all(&grok_dir).expect("grok dir");

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
                grok_home: None,
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
            ("auth.json", grok_dir.join("auth.json")),
            ("auth.json.lock", grok_dir.join("auth.json.lock")),
            (
                "auth.json.tmp.<pid>.<ts>",
                grok_dir.join("auth.json.tmp.7969.1778210964004"),
            ),
            (
                "mcp_credentials.json",
                grok_dir.join("mcp_credentials.json"),
            ),
            (
                "mcp_auth_<name>.lock",
                grok_dir.join("mcp_auth_linear.lock"),
            ),
        ] {
            let status = Command::new(sandbox_exec_path_for_test())
                .arg("-f")
                .arg(profile_file.path())
                .arg("/bin/sh")
                .arg("-c")
                .arg(format!("echo ok > {}", shell_escape(&target)))
                .status()
                .expect("run sandbox-exec");
            assert!(
                status.success(),
                "expected write to synthetic Grok {label} to succeed; status={status:?}"
            );
            assert!(
                target.exists(),
                "{label} target file was not written: {target:?}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn compiled_profile_allows_writes_to_claude_home_json_siblings() {
        // T20260508-13: Claude Code persists `$HOME/.claude.json` (plus
        // `.lock` and atomic-write `.tmp.<pid>.<ms_ts>` siblings) at the home
        // root, not under `$HOME/.claude/`. Without explicit allows the
        // kernel denies these writes and Claude hangs on its own lockfile
        // under sandbox-exec.
        use std::process::Command;

        if !sandbox_exec_can_apply() {
            return;
        }

        let parent = sandbox_test_parent("claude-home-json");
        let _cleanup = ScopeGuard(parent.clone());
        let synthetic_home = tempfile::Builder::new()
            .prefix("synthetic-home-")
            .tempdir_in(&parent)
            .expect("synthetic home tempdir");

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
                grok_home: None,
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
            (".claude.json", synthetic_home.path().join(".claude.json")),
            (
                ".claude.json.lock",
                synthetic_home.path().join(".claude.json.lock"),
            ),
            (
                ".claude.json.tmp.<pid>.<ts>",
                synthetic_home
                    .path()
                    .join(".claude.json.tmp.7969.1778210964004"),
            ),
        ] {
            let status = Command::new(sandbox_exec_path_for_test())
                .arg("-f")
                .arg(profile_file.path())
                .arg("/bin/sh")
                .arg("-c")
                .arg(format!("echo ok > {}", shell_escape(&target)))
                .status()
                .expect("run sandbox-exec");
            assert!(
                status.success(),
                "expected write to synthetic {label} to succeed; status={status:?}"
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
    fn sandbox_exec_path_for_test() -> PathBuf {
        sandbox_exec_path().expect("trusted sandbox-exec path")
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

        std::process::Command::new(sandbox_exec_path_for_test())
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
        fn default_write_allow_roots() -> Vec<PathBuf> {
            let mut roots = vec![
                PathBuf::from("/tmp"),
                PathBuf::from("/private/tmp"),
                PathBuf::from("/private/var/folders"),
                PathBuf::from("/dev"),
            ];
            let home = std::env::var_os("HOME");
            let codex_home = std::env::var_os("CODEX_HOME");
            let claude_config_dir = std::env::var_os("CLAUDE_CONFIG_DIR");
            let grok_home = std::env::var_os("GROK_HOME");
            if let Some(home) = non_empty_env_path(home.as_deref()) {
                roots.push(home.join("Library/Caches"));
                roots.push(home.join(".orbit/state/logs"));
            }
            roots.extend(provider_state_dirs(
                home.as_deref(),
                codex_home.as_deref(),
                claude_config_dir.as_deref(),
                grok_home.as_deref(),
            ));
            roots
        }

        fn matches_allowed(path: &Path, roots: &[PathBuf]) -> bool {
            roots.iter().any(|root| path.starts_with(root))
        }

        let allowed_roots = default_write_allow_roots();
        if matches_allowed(path, &allowed_roots) {
            return true;
        }
        match path.canonicalize() {
            Ok(canonical) => matches_allowed(&canonical, &allowed_roots),
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
