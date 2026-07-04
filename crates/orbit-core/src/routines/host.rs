//! Host identity for routine scheduling [ORB-10021].
//!
//! `hosts:` pinning in routine definitions matches against a `host_id` that
//! lives in `~/.orbit/host.toml` — the one genuinely host-local datum in the
//! routines design (ADR-0205 keeps discovery in the workspace registry;
//! `host.toml` survives only to carry identity). When the file is absent the
//! machine hostname is used, so pinning works out of the box on hosts whose
//! hostnames are already stable names like `dk-server-1`.

use std::fs;
use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;
use serde::Deserialize;

/// File under the global Orbit root carrying the host identity.
pub const HOST_TOML_FILE: &str = "host.toml";

#[derive(Debug, Deserialize)]
struct HostToml {
    host_id: Option<String>,
}

/// Resolve this machine's routine-scheduling identity: `host_id` from
/// `<global_root>/host.toml` when present and non-empty, otherwise the
/// machine hostname.
pub fn resolve_host_id(global_root: &Path) -> Result<String, OrbitError> {
    let path = host_toml_path(global_root);
    if path.exists() {
        let raw = fs::read_to_string(&path).map_err(|error| {
            OrbitError::Io(format!("failed to read '{}': {error}", path.display()))
        })?;
        let parsed: HostToml = toml::from_str(&raw).map_err(|error| {
            OrbitError::InvalidInput(format!("invalid host config '{}': {error}", path.display()))
        })?;
        if let Some(host_id) = parsed
            .host_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Ok(host_id.to_string());
        }
    }
    hostname_fallback()
}

/// Write (or overwrite) the host identity. Returns the file path written.
pub fn write_host_id(global_root: &Path, host_id: &str) -> Result<PathBuf, OrbitError> {
    let trimmed = host_id.trim();
    if trimmed.is_empty() {
        return Err(OrbitError::InvalidInput(
            "host_id must not be empty".to_string(),
        ));
    }
    let path = host_toml_path(global_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| OrbitError::Io(error.to_string()))?;
    }
    let body = format!(
        "# Host identity for routine scheduling (matched against `hosts:` in\n\
         # routine definitions). Written by `orbit routine init` [ORB-10021].\n\
         host_id = \"{trimmed}\"\n"
    );
    fs::write(&path, body).map_err(|error| {
        OrbitError::Io(format!("failed to write '{}': {error}", path.display()))
    })?;
    Ok(path)
}

fn host_toml_path(global_root: &Path) -> PathBuf {
    global_root.join(HOST_TOML_FILE)
}

fn hostname_fallback() -> Result<String, OrbitError> {
    let name = hostname::get()
        .map_err(|error| OrbitError::Io(format!("failed to resolve hostname: {error}")))?;
    let name = name.to_string_lossy().trim().to_string();
    if name.is_empty() {
        return Err(OrbitError::InvalidInput(
            "machine hostname is empty; set host_id in ~/.orbit/host.toml \
             via `orbit routine init --host-id <id>`"
                .to_string(),
        ));
    }
    Ok(name)
}
