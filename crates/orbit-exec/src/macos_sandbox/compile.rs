use std::ffi::OsStr;

use orbit_common::OrbitError;
use orbit_types::policy::ResolvedFsProfile;
use orbit_types::workflow::Provider;

/// Compile a [`ResolvedFsProfile`] into SBPL text suitable for
/// `sandbox-exec -f`.
///
/// `provider` is the canonical provider name of the CLI this profile will
/// confine (`claude`, `codex`, ...). It only selects the credential-store
/// carve-out described in [`provider_reads_macos_login_keychain`]; every other
/// clause is provider-independent. An unknown name compiles with the full
/// default credential denylist, so a typo fails closed.
///
/// The emitted profile:
/// - denies everything by default;
/// - allows broad reads (`file-read*`) for agent CLI compatibility, then
///   appends default read denies for well-known credential locations so those
///   paths still lose under SBPL's last-match-wins evaluation, and finally
///   re-allows the confined provider's own credential store if it lives in one
///   of those locations;
/// - allows the syscall classes agent CLIs rely on (process, signal, mach,
///   ipc, sysctl, iokit) and unrestricted network — agents call out to
///   provider APIs;
/// - allows writes inside the resolved `modify` scope plus a small set of
///   well-known scratch areas (`/tmp`, `/private/tmp`,
///   `/private/var/folders`, `~/Library/Caches`, and the HOME-derived Orbit
///   JSONL log directory) that tools and the filesystem layer expect to write to;
/// - emits resolved `read` / `modify` rules in order, including explicit
///   `(deny ...)` clauses for negated entries and narrow host-policy or
///   runtime re-allows after their enclosing deny, preserving SBPL's
///   last-match-wins evaluation.
///
/// Paths in `rules.modify` are emitted as-is. Callers must resolve
/// workspace-relative globs to absolute paths before invoking this
/// function — a relative `subpath` is meaningless to the kernel.
pub fn compile_macos_sandbox_profile(
    rules: &ResolvedFsProfile,
    provider: &str,
) -> Result<String, OrbitError> {
    let home = std::env::var_os("HOME");
    let codex_home = std::env::var_os("CODEX_HOME");
    let claude_config_dir = std::env::var_os("CLAUDE_CONFIG_DIR");
    let grok_home = std::env::var_os("GROK_HOME");
    compile_macos_sandbox_profile_with_env(
        rules,
        provider,
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
    provider: &str,
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
    emit_default_credential_read_denies(home, &mut out);
    emit_provider_credential_read_reallow(provider, home, &mut out);

    Ok(out)
}

/// Whether `provider`'s CLI reads its own credentials from the macOS login
/// Keychain, and therefore cannot run under the default Keychain read deny.
///
/// Only Claude Code does today: it keeps its OAuth session in the login
/// keychain item `Claude Code-credentials` and leaves
/// `~/.claude/.credentials.json` as an empty stub. Codex, Gemini, and Grok keep
/// credentials in plain files under their own state directories, which are
/// already granted, so they keep the deny. Names that do not resolve to a
/// canonical [`Provider`] keep the deny too — the carve-out fails closed.
/// [ORB-10929]
pub fn provider_reads_macos_login_keychain(provider: &str) -> bool {
    Provider::parse(provider).ok() == Some(Provider::Claude)
}

/// Re-allow the confined provider's own credential store after
/// [`emit_default_credential_read_denies`], so last-match-wins grants it.
///
/// Without this, a sandboxed `claude` cannot see its Keychain item and reports
/// `OAuth session expired and could not be refreshed` — an authentication
/// failure no re-login can clear, because the credential is present and simply
/// unreadable. The carve-out is deliberately narrow:
/// - it applies to one provider, so a Codex or Grok agent still cannot read any
///   keychain;
/// - it covers only the *user* keychain directory; `/Library/Keychains` and
///   `/System/Library/Keychains` stay denied for every provider;
/// - it grants reads only. Nothing here makes the login keychain writable, so a
///   sandboxed run can use a refreshed token in memory but cannot persist it
///   back to the keychain; re-authentication stays an unsandboxed operation.
///
/// Reading the keychain file is not the same as reading its secrets: item
/// contents stay encrypted and gated by their own per-item ACLs, which is why
/// the grant is scoped to the provider that owns the item it needs.
///
/// Precedence: this is the last clause in the profile, so it also outranks an
/// activity's own negated `read` rules. Nothing an activity declares can *widen*
/// the grant — it depends only on the confined provider — but an activity cannot
/// narrow it back either. Move this above the `rules.read` loop if a profile
/// ever needs to deny a provider its own credential store.
fn emit_provider_credential_read_reallow(provider: &str, home: Option<&OsStr>, out: &mut String) {
    if !provider_reads_macos_login_keychain(provider) {
        return;
    }
    let Some(home) = super::provider_dirs::non_empty_env_path(home) else {
        return;
    };
    let keychains = format!("{}/{USER_KEYCHAINS_SUBPATH}", home.display());
    out.push_str(&format!(
        "(allow file-read* (subpath \"{}\"))\n",
        super::sbpl_filter::sbpl_escape(&keychains)
    ));
}

/// HOME-relative path of the per-user keychain directory. Shared by the default
/// deny and the provider re-allow so the two clauses cannot drift.
const USER_KEYCHAINS_SUBPATH: &str = "Library/Keychains";

fn emit_default_credential_read_denies(home: Option<&OsStr>, out: &mut String) {
    if let Some(home) = super::provider_dirs::non_empty_env_path(home) {
        let home = home.display().to_string();
        for suffix in [
            ".ssh",
            ".aws",
            ".config/gh",
            USER_KEYCHAINS_SUBPATH,
            "Library/Application Support/Google/Chrome",
            "Library/Application Support/Chromium",
            "Library/Application Support/BraveSoftware/Brave-Browser",
            "Library/Application Support/Firefox",
        ] {
            emit_read_deny_subpath(&format!("{home}/{suffix}"), out);
        }
    }

    for path in ["/Library/Keychains", "/System/Library/Keychains"] {
        emit_read_deny_subpath(path, out);
    }
}

fn emit_read_deny_subpath(path: &str, out: &mut String) {
    out.push_str(&format!(
        "(deny file-read* (subpath \"{}\"))\n",
        super::sbpl_filter::sbpl_escape(path)
    ));
}
