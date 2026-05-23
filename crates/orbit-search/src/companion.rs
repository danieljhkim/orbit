//! Discovery of the installed search companion binary.
//!
//! `CompanionPaths` describes the on-disk layout under `~/.orbit/embed/`,
//! and `locate_companion()` resolves a callable path by checking, in order,
//! the standard install location, then a gated `ORBIT_SEARCH_COMPANION`
//! developer override. When both miss, the error is the actionable
//! `CompanionNotInstalled` shape so callers can surface a clean install hint.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;

pub(crate) const COMPANION_OVERRIDE_ENV: &str = "ORBIT_SEARCH_COMPANION";
pub(crate) const UNSAFE_COMPANION_OVERRIDE_ENV: &str = "ORBIT_SEARCH_COMPANION_ALLOW_UNSAFE";
pub const INSTALL_REMEDIATION: &str = "Semantic search not enabled. Run `orbit semantic install` to download the inference companion.";

#[derive(Debug, Clone)]
pub struct CompanionPaths {
    pub root: PathBuf,
    pub bin_dir: PathBuf,
    pub models_dir: PathBuf,
    pub active_model_path: PathBuf,
}

impl CompanionPaths {
    pub fn default_under_home() -> Result<Self, OrbitError> {
        let root = home_dir()
            .ok_or_else(|| OrbitError::InvalidInput("HOME/USERPROFILE is not set".to_string()))?
            .join(".orbit")
            .join("embed");
        Ok(Self::new(root))
    }

    pub fn new(root: PathBuf) -> Self {
        Self {
            bin_dir: root.join("bin"),
            models_dir: root.join("models"),
            active_model_path: root.join("active-model"),
            root,
        }
    }

    pub fn companion_path(&self) -> PathBuf {
        self.bin_dir.join(platform_companion_filename())
    }

    pub fn model_dir(&self, model_id: &str) -> PathBuf {
        self.models_dir.join(model_id)
    }
}

pub fn platform_companion_filename() -> String {
    if cfg!(windows) {
        format!("orbit-search-companion-{}.exe", platform_id())
    } else {
        format!("orbit-search-companion-{}", platform_id())
    }
}

pub fn platform_id() -> &'static str {
    match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => "macos-aarch64",
        ("macos", "x86_64") => "macos-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("linux", "x86_64") => "linux-x86_64",
        ("windows", "x86_64") => "windows-x86_64",
        _ => "unknown",
    }
}

pub fn locate_companion() -> Result<PathBuf, OrbitError> {
    if let Ok(paths) = CompanionPaths::default_under_home() {
        let standard = paths.companion_path();
        if is_executable_file(&standard) {
            return Ok(standard);
        }
    }

    if let Ok(path) = env::var(COMPANION_OVERRIDE_ENV) {
        let path = PathBuf::from(path);
        validate_companion_override_path(&path)?;
        return Ok(path);
    }

    Err(OrbitError::CompanionNotInstalled(
        INSTALL_REMEDIATION.to_string(),
    ))
}

pub(crate) fn unsafe_companion_overrides_enabled() -> bool {
    env::var(UNSAFE_COMPANION_OVERRIDE_ENV)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

pub(crate) fn validate_companion_override_path(path: &Path) -> Result<(), OrbitError> {
    if !unsafe_companion_overrides_enabled() {
        return Err(OrbitError::InvalidInput(format!(
            "{COMPANION_OVERRIDE_ENV} is a developer-only override; set {UNSAFE_COMPANION_OVERRIDE_ENV}=1 after verifying the companion path is trusted"
        )));
    }
    if !path.is_absolute() {
        return Err(OrbitError::InvalidInput(format!(
            "{COMPANION_OVERRIDE_ENV} must be an absolute path: {}",
            path.display()
        )));
    }
    validate_companion_path(path, CompanionPathTrust::DeveloperOverride)
}

pub(crate) fn validate_managed_companion_path(path: &Path) -> Result<(), OrbitError> {
    validate_companion_path(path, CompanionPathTrust::ManagedInstall)
}

fn is_executable_file(path: &Path) -> bool {
    validate_managed_companion_path(path).is_ok()
}

#[derive(Debug, Clone, Copy)]
enum CompanionPathTrust {
    ManagedInstall,
    DeveloperOverride,
}

fn validate_companion_path(path: &Path, trust: CompanionPathTrust) -> Result<(), OrbitError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        OrbitError::InvalidInput(format!(
            "search companion path is not readable at {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.file_type().is_file() {
        return Err(OrbitError::InvalidInput(format!(
            "search companion path must be a regular file: {}",
            path.display()
        )));
    }
    validate_executable_metadata(path, &metadata)?;
    if matches!(trust, CompanionPathTrust::DeveloperOverride) {
        validate_developer_override_metadata(path, &metadata)?;
    }
    Ok(())
}

#[cfg(unix)]
fn validate_executable_metadata(path: &Path, metadata: &fs::Metadata) -> Result<(), OrbitError> {
    use std::os::unix::fs::PermissionsExt;

    if metadata.permissions().mode() & 0o111 == 0 {
        return Err(OrbitError::InvalidInput(format!(
            "search companion is not executable: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_executable_metadata(_path: &Path, _metadata: &fs::Metadata) -> Result<(), OrbitError> {
    Ok(())
}

#[cfg(unix)]
fn validate_developer_override_metadata(
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<(), OrbitError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let effective_uid = {
        // SAFETY: geteuid has no preconditions and only reads the process effective uid.
        unsafe { libc::geteuid() }
    };
    if metadata.uid() != effective_uid {
        return Err(OrbitError::InvalidInput(format!(
            "search companion override must be owned by the current user: {}",
            path.display()
        )));
    }
    if metadata.permissions().mode() & 0o022 != 0 {
        return Err(OrbitError::InvalidInput(format!(
            "search companion override must not be group/world writable: {}",
            path.display()
        )));
    }
    let parent = path.parent().ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "search companion override has no parent directory: {}",
            path.display()
        ))
    })?;
    let parent_metadata = fs::symlink_metadata(parent).map_err(|error| {
        OrbitError::InvalidInput(format!(
            "search companion override parent is not readable at {}: {error}",
            parent.display()
        ))
    })?;
    if parent_metadata.uid() != effective_uid {
        return Err(OrbitError::InvalidInput(format!(
            "search companion override parent must be owned by the current user: {}",
            parent.display()
        )));
    }
    if parent_metadata.permissions().mode() & 0o022 != 0 {
        return Err(OrbitError::InvalidInput(format!(
            "search companion override parent must not be group/world writable: {}",
            parent.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_developer_override_metadata(
    _path: &Path,
    _metadata: &fs::Metadata,
) -> Result<(), OrbitError> {
    Ok(())
}

fn home_dir() -> Option<PathBuf> {
    env::var("HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var("USERPROFILE")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from)
        })
}
