//! Discovery of the installed embedding companion binary.
//!
//! `CompanionPaths` describes the on-disk layout under `~/.orbit/embed/`,
//! and `locate_companion()` resolves a callable path by checking, in order,
//! the `ORBIT_EMBED_COMPANION` env override, the standard install location,
//! then `$PATH`. When all three miss, the error is the actionable
//! `CompanionNotInstalled` shape so callers can surface a clean install hint.

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;

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
        format!("orbit-embed-companion-{}.exe", platform_id())
    } else {
        format!("orbit-embed-companion-{}", platform_id())
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
    if let Ok(path) = env::var("ORBIT_EMBED_COMPANION") {
        let path = PathBuf::from(path);
        if is_executable_file(&path) {
            return Ok(path);
        }
    }

    if let Ok(paths) = CompanionPaths::default_under_home() {
        let standard = paths.companion_path();
        if is_executable_file(&standard) {
            return Ok(standard);
        }
    }

    for name in path_candidate_names() {
        if let Some(path) = find_on_path(&name) {
            return Ok(path);
        }
    }

    Err(OrbitError::CompanionNotInstalled(
        INSTALL_REMEDIATION.to_string(),
    ))
}

fn path_candidate_names() -> Vec<OsString> {
    let mut names = vec![OsString::from("orbit-embed-companion")];
    names.push(OsString::from(platform_companion_filename()));
    if cfg!(windows) {
        names.push(OsString::from("orbit-embed-companion.exe"));
    }
    names
}

fn find_on_path(name: &OsString) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    env::split_paths(&paths)
        .map(|dir| dir.join(name))
        .find(|path| is_executable_file(path))
}

fn is_executable_file(path: &Path) -> bool {
    path.is_file()
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
