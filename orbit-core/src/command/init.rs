use std::fs;
use std::path::{Path, PathBuf};

use orbit_types::OrbitError;

use crate::OrbitRuntime;
use crate::command::job::seed_default_jobs;

const DEFAULT_IDENTITIES: [(&str, &str); 6] = [
    ("linus", include_str!("../../assets/identities/linus.yaml")),
    ("john", include_str!("../../assets/identities/john.yaml")),
    ("kent", include_str!("../../assets/identities/kent.yaml")),
    ("rob", include_str!("../../assets/identities/rob.yaml")),
    ("grace", include_str!("../../assets/identities/grace.yaml")),
    ("steve", include_str!("../../assets/identities/steve.yaml")),
];
const DEFAULT_SKILLS: [(&str, &str); 6] = [
    (
        "orbit-approve-task",
        include_str!("../../assets/skills/orbit-approve-task/SKILL.md"),
    ),
    (
        "orbit-assess-codebase",
        include_str!("../../assets/skills/orbit-assess-codebase/SKILL.md"),
    ),
    (
        "orbit-execute-change-request",
        include_str!("../../assets/skills/orbit-execute-change-request/SKILL.md"),
    ),
    (
        "orbit-maintain-system",
        include_str!("../../assets/skills/orbit-maintain-system/SKILL.md"),
    ),
    (
        "orbit-manage-tasks",
        include_str!("../../assets/skills/orbit-manage-tasks/SKILL.md"),
    ),
    (
        "orbit-track-issues",
        include_str!("../../assets/skills/orbit-track-issues/SKILL.md"),
    ),
];
const DEFAULT_CONFIG_TEMPLATE: &str = include_str!("../../assets/config/default-config.toml");
const DEFAULT_CONFIG_TEMPLATE_REPO: &str =
    include_str!("../../assets/config/default-config-repo.toml");

#[derive(Debug, Clone)]
pub struct InitResult {
    pub created_identity_files: usize,
    pub identity_root: String,
    pub created_skill_files: usize,
    pub skills_root: String,
    pub created_skills_symlink: bool,
    pub created_config: bool,
    pub config_path: String,
    pub created_default_work: bool,
}

#[derive(Debug, Clone, Default)]
pub struct InitOptions {
    pub force: bool,
}

impl OrbitRuntime {
    pub fn init_workspace(&self) -> Result<InitResult, OrbitError> {
        self.init_workspace_with_options(InitOptions::default())
    }

    pub fn init_workspace_with_options(
        &self,
        options: InitOptions,
    ) -> Result<InitResult, OrbitError> {
        let data_root = self.data_root();
        let orbit_home = OrbitRuntime::orbit_home_root();

        if data_root == orbit_home {
            let cwd = std::env::current_dir().map_err(|e| OrbitError::Io(e.to_string()))?;
            if let Some(repo_root) = find_git_repo_root(&cwd) {
                return init_workspace_at_root(&repo_root.join(".orbit"), options);
            }
        }

        init_workspace_at_root(&data_root, options)
    }
}

pub(crate) fn ensure_orbit_root_initialized(orbit_root: &Path) -> Result<(), OrbitError> {
    let _ = init_workspace_at_root(orbit_root, InitOptions::default())?;
    Ok(())
}

fn init_workspace_at_root(
    orbit_root: &Path,
    options: InitOptions,
) -> Result<InitResult, OrbitError> {
    let init_target = resolve_init_target_from_root(orbit_root)?;
    let orbit_root = init_target.orbit_root.clone();
    let identity_root = orbit_root.join("identities");

    if options.force {
        remove_path_if_exists(&orbit_root)?;
    }
    fs::create_dir_all(&orbit_root).map_err(|e| OrbitError::Io(e.to_string()))?;
    fs::create_dir_all(&identity_root).map_err(|e| OrbitError::Io(e.to_string()))?;
    let skills_root = orbit_root.join("skills");
    fs::create_dir_all(&skills_root).map_err(|e| OrbitError::Io(e.to_string()))?;

    let mut created = 0usize;
    for (name, content) in DEFAULT_IDENTITIES {
        let path = identity_root.join(format!("{name}.yaml"));
        if path.exists() {
            continue;
        }
        write_identity_file(&path, content)?;
        created += 1;
    }

    let mut created_skill_files = 0usize;
    for (id, content) in DEFAULT_SKILLS {
        let path = skills_root.join(id).join("SKILL.md");
        if path.exists() {
            continue;
        }
        write_identity_file(&path, content)?;
        created_skill_files += 1;
    }

    let config_path = orbit_root.join("config.toml");
    let created_config = if config_path.exists() {
        false
    } else {
        write_identity_file(&config_path, init_target.config_template)?;
        true
    };

    let skill_ids = DEFAULT_SKILLS.map(|(id, _)| id);
    let created_skills_symlink = ensure_skill_links(
        &skills_root,
        &skill_ids,
        &init_target.skills_links_root,
        options.force,
    )?;

    let init_runtime = OrbitRuntime::from_data_root(&orbit_root)?;
    let created_default_work = seed_default_jobs(&init_runtime)? > 0;

    Ok(InitResult {
        created_identity_files: created,
        identity_root: identity_root.to_string_lossy().to_string(),
        created_skill_files,
        skills_root: skills_root.to_string_lossy().to_string(),
        created_skills_symlink,
        created_config,
        config_path: config_path.to_string_lossy().to_string(),
        created_default_work,
    })
}

fn write_identity_file(path: &Path, content: &str) -> Result<(), OrbitError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;
    }
    fs::write(path, content).map_err(|e| OrbitError::Io(e.to_string()))
}

fn home_orbit_root() -> Result<PathBuf, OrbitError> {
    Ok(home_dir()?.join(".orbit"))
}

#[derive(Debug, Clone)]
struct InitTarget {
    orbit_root: PathBuf,
    skills_links_root: PathBuf,
    config_template: &'static str,
}

fn resolve_init_target_from_root(orbit_root: &Path) -> Result<InitTarget, OrbitError> {
    let orbit_root = orbit_root.to_path_buf();
    let home_root = home_orbit_root()?;
    let config_template = if orbit_root == home_root {
        DEFAULT_CONFIG_TEMPLATE
    } else {
        DEFAULT_CONFIG_TEMPLATE_REPO
    };

    let skills_links_root = if let Some(repo_root) = find_git_repo_root(&orbit_root) {
        repo_root.join(".agents").join("skills")
    } else {
        home_dir()?.join(".agents").join("skills")
    };

    Ok(InitTarget {
        orbit_root,
        skills_links_root,
        config_template,
    })
}

fn find_git_repo_root(start: &Path) -> Option<PathBuf> {
    for ancestor in start.ancestors() {
        if ancestor.join(".git").exists() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

fn home_dir() -> Result<PathBuf, OrbitError> {
    if let Ok(home) = std::env::var("HOME")
        && !home.trim().is_empty()
    {
        return Ok(PathBuf::from(home));
    }
    if let Ok(profile) = std::env::var("USERPROFILE")
        && !profile.trim().is_empty()
    {
        return Ok(PathBuf::from(profile));
    }
    Err(OrbitError::InvalidInput(
        "HOME/USERPROFILE is not set; cannot resolve home directory".to_string(),
    ))
}

fn ensure_skill_links(
    skills_root: &Path,
    skill_ids: &[&str],
    skills_links_dir: &Path,
    force: bool,
) -> Result<bool, OrbitError> {
    if let Some(parent) = skills_links_dir.parent() {
        fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;
    }

    if let Ok(metadata) = fs::symlink_metadata(skills_links_dir) {
        if metadata.file_type().is_symlink() {
            // Migrate old behavior (~/.agents/skills -> ~/.orbit/skills) to
            // per-skill symlink entries.
            fs::remove_file(skills_links_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
        } else if !metadata.file_type().is_dir() {
            if force {
                remove_path_if_exists(skills_links_dir)?;
            } else {
                return Err(OrbitError::InvalidInput(format!(
                    "expected '{}' to be a directory for skill links; found non-directory path",
                    skills_links_dir.display()
                )));
            }
        }
    }

    if !skills_links_dir.exists() {
        fs::create_dir_all(skills_links_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
    } else if !skills_links_dir.is_dir() {
        if force {
            remove_path_if_exists(skills_links_dir)?;
            fs::create_dir_all(skills_links_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
        } else {
            return Err(OrbitError::InvalidInput(format!(
                "expected '{}' to be a directory for skill links; found non-directory path",
                skills_links_dir.display()
            )));
        }
    }
    let canonical_skills_root = skills_root
        .canonicalize()
        .map_err(|e| OrbitError::Io(e.to_string()))?;

    let mut changed = false;
    for skill_id in skill_ids {
        let target = skills_root.join(skill_id);
        if !target.exists() {
            return Err(OrbitError::InvalidInput(format!(
                "skill target does not exist for link: {}",
                target.display()
            )));
        }
        let link_path = skills_links_dir.join(skill_id);

        if let Ok(link_meta) = fs::symlink_metadata(&link_path) {
            if link_meta.file_type().is_symlink() {
                let target_path =
                    fs::read_link(&link_path).map_err(|e| OrbitError::Io(e.to_string()))?;
                let resolved_target = if target_path.is_absolute() {
                    target_path
                } else {
                    link_path
                        .parent()
                        .unwrap_or(Path::new("."))
                        .join(target_path)
                        .to_path_buf()
                };
                let canonical_expected = canonical_skills_root.join(skill_id);
                if let Ok(canonical_existing) = resolved_target.canonicalize()
                    && canonical_existing == canonical_expected
                {
                    continue;
                }
                fs::remove_file(&link_path).map_err(|e| OrbitError::Io(e.to_string()))?;
                create_dir_symlink(&target, &link_path)?;
                changed = true;
                continue;
            }
            if force {
                remove_path_if_exists(&link_path)?;
                create_dir_symlink(&target, &link_path)?;
                changed = true;
                continue;
            }
            return Err(OrbitError::InvalidInput(format!(
                "expected '{}' to be a symlink to '{}'; found non-symlink path",
                link_path.display(),
                target.display()
            )));
        }

        create_dir_symlink(&target, &link_path)?;
        changed = true;
    }

    Ok(changed)
}

#[cfg(unix)]
fn create_dir_symlink(src: &Path, dst: &Path) -> Result<(), OrbitError> {
    std::os::unix::fs::symlink(src, dst).map_err(|e| OrbitError::Io(e.to_string()))
}

#[cfg(windows)]
fn create_dir_symlink(src: &Path, dst: &Path) -> Result<(), OrbitError> {
    std::os::windows::fs::symlink_dir(src, dst).map_err(|e| OrbitError::Io(e.to_string()))
}

fn remove_path_if_exists(path: &Path) -> Result<(), OrbitError> {
    if !path.exists() {
        return Ok(());
    }

    let metadata = fs::symlink_metadata(path).map_err(|e| OrbitError::Io(e.to_string()))?;
    if metadata.file_type().is_symlink() {
        fs::remove_file(path).map_err(|e| OrbitError::Io(e.to_string()))
    } else if metadata.is_dir() {
        fs::remove_dir_all(path).map_err(|e| OrbitError::Io(e.to_string()))
    } else {
        fs::remove_file(path).map_err(|e| OrbitError::Io(e.to_string()))
    }
}
