use std::fs;
use std::path::{Path, PathBuf};

use orbit_types::OrbitError;

use crate::OrbitRuntime;
use crate::command::activity::seed_default_activities;
use crate::command::identity::seed_default_identities;
use crate::command::job::seed_default_jobs;
use crate::command::skill::{default_skill_ids, seed_default_skills};
use crate::config::seed_default_config;
use crate::fs_utils::{create_dir_symlink, remove_path_if_exists};

const BUILTIN_ACTIVITY_RENAMES: &[(&str, &str)] = &[
    ("approve-task-leader", "review_tasks"),
    ("approve_task_leader", "review_tasks"),
    ("oversee-orbit-operations", "oversee_orbit_operations"),
    ("perform-maintenance", "perform_maintenance"),
    ("triage-and-dispatch-task", "dispatch_task"),
];

const BUILTIN_JOB_RENAMES: &[(&str, &str)] = &[
    ("job-approve-task-leader", "job_review_tasks"),
    ("job_approve_task_leader", "job_review_tasks"),
    (
        "job-oversee-orbit-operations",
        "job_oversee_orbit_operations",
    ),
    ("job-perform-maintenance", "job_perform_maintenance"),
    ("job-execute-task", "job_task_pipeline"),
    ("job_execute_task", "job_task_pipeline"),
];

#[derive(Debug, Clone)]
pub struct InitResult {
    pub refreshed_identity_files: usize,
    pub identity_root: String,
    pub refreshed_skill_files: usize,
    pub skills_root: String,
    pub created_skills_symlink: bool,
    pub created_config: bool,
    pub config_path: String,
    pub refreshed_default_activities: usize,
    pub refreshed_default_jobs: usize,
}

#[derive(Debug, Clone, Default)]
pub struct InitOptions {
    pub force: bool,
    /// When true, always overwrite default identity and skill files even if
    /// they already exist.  Explicit `orbit init` sets this; implicit
    /// bootstrap from other commands does not.
    pub refresh_defaults: bool,
}

impl OrbitRuntime {
    pub fn init_workspace(&self) -> Result<InitResult, OrbitError> {
        self.init_workspace_with_options(InitOptions::default())
    }

    pub fn init_workspace_with_options(
        &self,
        options: InitOptions,
    ) -> Result<InitResult, OrbitError> {
        init_workspace_at_root(&self.data_root(), options)
    }
}

pub(crate) fn ensure_orbit_root_initialized(orbit_root: &Path) -> Result<(), OrbitError> {
    let _ = init_workspace_at_root(orbit_root, InitOptions::default())?;
    Ok(())
}

pub fn init_workspace_from_root_override(
    root_override: Option<&Path>,
    options: InitOptions,
) -> Result<InitResult, OrbitError> {
    let cwd = std::env::current_dir().map_err(|e| OrbitError::Io(e.to_string()))?;
    let data_root = crate::runtime::resolve_initialize_data_root(&cwd, root_override)?;
    init_workspace_at_root(&data_root, options)
}

fn init_workspace_at_root(
    orbit_root: &Path,
    options: InitOptions,
) -> Result<InitResult, OrbitError> {
    let init_target = resolve_init_target_from_root(orbit_root);
    let orbit_root = init_target.orbit_root.clone();
    let identity_root = orbit_root.join("identities");

    if options.force {
        remove_path_if_exists(&orbit_root)?;
    }
    fs::create_dir_all(&orbit_root).map_err(|e| OrbitError::Io(e.to_string()))?;
    fs::create_dir_all(&identity_root).map_err(|e| OrbitError::Io(e.to_string()))?;
    let skills_root = orbit_root.join("skills");
    fs::create_dir_all(&skills_root).map_err(|e| OrbitError::Io(e.to_string()))?;
    migrate_builtin_names_to_snake_case(&orbit_root)?;

    let overwrite = options.force || options.refresh_defaults;
    let refreshed_identity_files = seed_default_identities(&identity_root, overwrite)?;
    let refreshed_skill_files = seed_default_skills(&skills_root, &orbit_root, overwrite)?;
    let config_path = orbit_root.join("config.toml");
    let created_config = seed_default_config(&config_path)?;

    let skill_ids = default_skill_ids();
    let mut created_skills_symlink = false;
    for skills_links_root in &init_target.skills_links_roots {
        created_skills_symlink |=
            ensure_skill_links(&skills_root, &skill_ids, skills_links_root, options.force)?;
    }

    let init_runtime = OrbitRuntime::from_data_root(&orbit_root)?;
    let refreshed_default_activities = seed_default_activities(&init_runtime, overwrite)?;
    let refreshed_default_jobs = seed_default_jobs(&init_runtime, overwrite)?;

    Ok(InitResult {
        refreshed_identity_files,
        identity_root: identity_root.to_string_lossy().to_string(),
        refreshed_skill_files,
        skills_root: skills_root.to_string_lossy().to_string(),
        created_skills_symlink,
        created_config,
        config_path: config_path.to_string_lossy().to_string(),
        refreshed_default_activities,
        refreshed_default_jobs,
    })
}

fn migrate_builtin_names_to_snake_case(orbit_root: &Path) -> Result<(), OrbitError> {
    for activity_dir in [
        orbit_root.join("activities").join("active"),
        orbit_root.join("activities").join("inactive"),
    ] {
        migrate_named_yaml_files(&activity_dir, BUILTIN_ACTIVITY_RENAMES)?;
    }

    let jobs_root = orbit_root.join("jobs");
    for job_dir in [
        jobs_root.join("jobs"),
        jobs_root.join("jobs").join("disabled"),
    ] {
        migrate_named_yaml_files(&job_dir, BUILTIN_JOB_RENAMES)?;
    }

    for runs_root in [
        jobs_root.join("runs"),
        jobs_root.join("runs").join("archived"),
    ] {
        migrate_job_run_dirs(&runs_root)?;
    }

    Ok(())
}

fn migrate_named_yaml_files(dir: &Path, renames: &[(&str, &str)]) -> Result<(), OrbitError> {
    if !dir.exists() {
        return Ok(());
    }

    for (old_id, new_id) in renames {
        let old_path = dir.join(format!("{old_id}.yaml"));
        if !old_path.exists() {
            continue;
        }

        let new_path = dir.join(format!("{new_id}.yaml"));
        migrate_text_file(&old_path, &new_path)?;
    }

    Ok(())
}

fn migrate_text_file(old_path: &Path, new_path: &Path) -> Result<(), OrbitError> {
    let raw = fs::read_to_string(old_path).map_err(|e| OrbitError::Io(e.to_string()))?;
    let migrated = apply_builtin_name_replacements(&raw);
    if new_path.exists() {
        let current = fs::read_to_string(new_path).map_err(|e| OrbitError::Io(e.to_string()))?;
        if current != migrated {
            return Err(OrbitError::InvalidInput(format!(
                "conflicting built-in migration targets '{}' and '{}'",
                old_path.display(),
                new_path.display()
            )));
        }
        fs::remove_file(old_path).map_err(|e| OrbitError::Io(e.to_string()))?;
        return Ok(());
    }

    write_atomic(new_path, &migrated)?;
    fs::remove_file(old_path).map_err(|e| OrbitError::Io(e.to_string()))?;
    Ok(())
}

fn migrate_job_run_dirs(runs_root: &Path) -> Result<(), OrbitError> {
    if !runs_root.exists() {
        return Ok(());
    }

    for (old_job_id, new_job_id) in BUILTIN_JOB_RENAMES {
        let old_dir = runs_root.join(old_job_id);
        if !old_dir.exists() {
            continue;
        }

        let new_dir = runs_root.join(new_job_id);
        fs::create_dir_all(&new_dir).map_err(|e| OrbitError::Io(e.to_string()))?;

        let entries = fs::read_dir(&old_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
        for entry in entries {
            let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
            let run_dir = entry.path();
            if !run_dir.is_dir() {
                continue;
            }

            migrate_job_run_bundle(&run_dir)?;

            let run_name = run_dir.file_name().ok_or_else(|| {
                OrbitError::Io(format!("invalid run bundle path '{}'", run_dir.display()))
            })?;
            let destination = new_dir.join(run_name);
            if destination.exists() {
                return Err(OrbitError::InvalidInput(format!(
                    "conflicting migrated run bundle '{}'",
                    destination.display()
                )));
            }
            fs::rename(&run_dir, &destination).map_err(|e| OrbitError::Io(e.to_string()))?;
        }

        fs::remove_dir(&old_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
    }

    Ok(())
}

fn migrate_job_run_bundle(run_dir: &Path) -> Result<(), OrbitError> {
    let jrun_path = run_dir.join("jrun.yaml");
    if jrun_path.exists() {
        rewrite_file_in_place(&jrun_path)?;
    }

    let steps_dir = run_dir.join("steps");
    if !steps_dir.exists() {
        return Ok(());
    }

    let entries = fs::read_dir(&steps_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
        let step_path = entry.path();
        if !step_path.is_file() {
            continue;
        }
        rewrite_file_in_place(&step_path)?;

        let Some(file_name) = step_path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let migrated_name = apply_builtin_name_replacements(file_name);
        if migrated_name == file_name {
            continue;
        }

        let destination = step_path.with_file_name(migrated_name);
        if destination.exists() {
            return Err(OrbitError::InvalidInput(format!(
                "conflicting migrated step file '{}'",
                destination.display()
            )));
        }
        fs::rename(&step_path, &destination).map_err(|e| OrbitError::Io(e.to_string()))?;
    }

    Ok(())
}

fn rewrite_file_in_place(path: &Path) -> Result<(), OrbitError> {
    let raw = fs::read_to_string(path).map_err(|e| OrbitError::Io(e.to_string()))?;
    let migrated = apply_builtin_name_replacements(&raw);
    if migrated != raw {
        write_atomic(path, &migrated)?;
    }
    Ok(())
}

fn apply_builtin_name_replacements(raw: &str) -> String {
    let mut migrated = raw.to_string();
    for (old, new) in BUILTIN_JOB_RENAMES {
        migrated = migrated.replace(old, new);
    }
    for (old, new) in BUILTIN_ACTIVITY_RENAMES {
        migrated = migrated.replace(old, new);
    }
    migrated
}

fn write_atomic(path: &Path, content: &str) -> Result<(), OrbitError> {
    let parent = path.parent().ok_or_else(|| {
        OrbitError::Io(format!("cannot determine parent for '{}'", path.display()))
    })?;
    fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;

    let tmp_path = path.with_extension("yaml.tmp");
    fs::write(&tmp_path, content).map_err(|e| OrbitError::Io(e.to_string()))?;
    if let Err(err) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(OrbitError::Io(err.to_string()));
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct InitTarget {
    orbit_root: PathBuf,
    skills_links_roots: Vec<PathBuf>,
}

fn resolve_init_target_from_root(orbit_root: &Path) -> InitTarget {
    let orbit_root = orbit_root.to_path_buf();
    let skills_links_base = if let Some(repo_root) = find_git_repo_root(&orbit_root) {
        repo_root
    } else {
        orbit_root
            .parent()
            .unwrap_or(orbit_root.as_path())
            .to_path_buf()
    };
    let skills_links_roots = skill_link_roots(&skills_links_base);

    InitTarget {
        orbit_root,
        skills_links_roots,
    }
}

fn skill_link_roots(base_root: &Path) -> Vec<PathBuf> {
    [".agents", ".claude"]
        .into_iter()
        .map(|dir| base_root.join(dir).join("skills"))
        .collect()
}

fn find_git_repo_root(start: &Path) -> Option<PathBuf> {
    crate::paths::find_git_repo_root(start)
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
            // Migrate legacy root-level skill symlinks to per-skill entries.
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
