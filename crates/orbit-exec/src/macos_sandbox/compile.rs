use std::ffi::OsStr;

use orbit_common::types::{OrbitError, ResolvedFsProfile};

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
pub(super) struct SandboxCompileEnv<'a> {
    pub(super) home: Option<&'a OsStr>,
    pub(super) codex_home: Option<&'a OsStr>,
    pub(super) claude_config_dir: Option<&'a OsStr>,
    pub(super) grok_home: Option<&'a OsStr>,
}

pub(super) fn compile_macos_sandbox_profile_with_env(
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
    if let Some(home) = super::provider_dirs::non_empty_env_path(home) {
        let home = home.display().to_string();
        out.push_str(&format!(
            "(allow file-write* (subpath \"{}/Library/Caches\"))\n",
            super::sbpl_filter::sbpl_escape(&home)
        ));
        // The agent CLI inherits the sandbox into its `orbit mcp serve` child
        // (and any other `orbit ...` calls it makes). Logging initializes
        // before the child can resolve Orbit's runtime roots, so the profile
        // carries the one HOME-derived path that must be writable up front.
        // Runtime-specific store/artifact paths are appended by orbit-core's
        // sandbox resolver instead of granting the whole HOME/.orbit tree.
        out.push_str(&format!(
            "(allow file-write* (subpath \"{}/.orbit/state/logs\"))\n",
            super::sbpl_filter::sbpl_escape(&home)
        ));
    }
    // Per-provider state directories. Each `backend: cli` agent CLI writes
    // setup state (sessions, settings, history, etc.) before it reads
    // Orbit's envelope. Active provider is not threaded through SBPL
    // compilation, and per-provider allowances do not widen attack surface,
    // so emit narrow allows for every supported provider's state dir
    // unconditionally.
    for state_dir in
        super::provider_dirs::provider_state_dirs(home, codex_home, claude_config_dir, grok_home)
    {
        out.push_str(&format!(
            "(allow file-write* (subpath \"{}\"))\n",
            super::sbpl_filter::sbpl_escape(&state_dir.display().to_string())
        ));
    }
    super::provider_dirs::emit_claude_home_json_allows(home, claude_config_dir, &mut out);
    super::provider_dirs::emit_grok_state_file_allows(home, grok_home, &mut out);

    for rule in &rules.modify {
        if let Some(deny_path) = rule.strip_prefix('!') {
            out.push_str(&format!(
                "(deny file-write* {})\n",
                super::sbpl_filter::sbpl_filter_for_deny_rule(deny_path)
            ));
            continue;
        }
        out.push_str(&format!(
            "(allow file-write* {})\n",
            super::sbpl_filter::sbpl_filter_for_allow_rule(rule)
        ));
    }

    for rule in &rules.read {
        if let Some(deny_path) = rule.strip_prefix('!') {
            out.push_str(&format!(
                "(deny file-read* {})\n",
                super::sbpl_filter::sbpl_filter_for_deny_rule(deny_path)
            ));
        }
    }

    Ok(out)
}
