use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::sbpl_filter::{push_regex_escaped, push_regex_escaped_str, sbpl_escape};

pub(super) fn provider_state_dirs(
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

pub(super) fn non_empty_env_path(value: Option<&OsStr>) -> Option<PathBuf> {
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
pub(super) fn emit_claude_home_json_allows(
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
pub(super) fn emit_grok_state_file_allows(
    home: Option<&OsStr>,
    grok_home: Option<&OsStr>,
    out: &mut String,
) {
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

#[cfg(test)]
mod tests;
