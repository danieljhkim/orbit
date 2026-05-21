use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use orbit_common::types::{OrbitError, WorkspacePaths};
use orbit_store::{friction_store, global_executor_def_store, global_policy_def_store};
use regex::{Captures, Regex};

use crate::OrbitRuntime;
use crate::command::activity::seed_default_activities;
use crate::command::executor::seed_default_executors;
use crate::command::job::seed_default_jobs;
use crate::command::policy::seed_default_policies;
use crate::command::skill::{
    default_skill_ids, is_default_skill_file_for_root, seed_default_skills,
};
use orbit_common::utility::fs::{atomic_write_text, create_dir_symlink, remove_path_if_exists};

use crate::config::{RawAgentRoleConfig, RuntimeConfig, seed_default_config};
use crate::runtime::resolve_global_root;

const LEGACY_WORKSPACE_SEEDED_SKILL_IDS: [&str; 2] = ["orbit-approve-task", "orbit-pr"];
const DESIGN_CONVENTIONS_TEMPLATE: &str = include_str!("../../../../docs/design/CONVENTIONS.md");
const DESIGN_DIR: &str = "docs/design";

#[derive(Debug, Clone)]
pub struct InitResult {
    pub refreshed_skill_files: usize,
    pub created_skills_symlink: bool,
    pub created_config: bool,
    pub refreshed_default_activities: usize,
    pub refreshed_default_jobs: usize,
    pub refreshed_default_executors: usize,
    pub refreshed_default_policies: usize,
}

#[derive(Debug, Clone, Default)]
pub struct InitOptions {
    pub force: bool,
    /// When true, always overwrite default skill files even if
    /// they already exist.  Explicit `orbit init` sets this; implicit
    /// bootstrap from other commands does not.
    pub refresh_defaults: bool,
    /// When true, seed only the globally scoped resource sets and skip
    /// workspace-local layout concerns like skills, tasks, and state.
    pub global_only: bool,
    /// Explicit global root to seed when preparing a workspace root.
    pub global_root_override: Option<PathBuf>,
    /// When true, create/update user-level skill symlinks for global skills.
    pub link_global_skills: bool,
    /// Per-role agent settings to embed in the freshly seeded `config.toml`
    /// as a `[crews.custom]` table. Keyed by role name (`reviewer`,
    /// `implementer`, `planner`). `None` and an empty map both mean "use
    /// the default crew template". Ignored when config.toml already exists
    /// — init remains idempotent.
    pub role_settings: Option<BTreeMap<String, RawAgentRoleConfig>>,
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

/// Ensures both global and workspace roots are bootstrapped.
/// Global root gets config plus all globally scoped resource defaults.
/// Workspace root gets only workspace-local layout and runtime state dirs.
pub(crate) fn ensure_orbit_root_initialized(
    global_root: &Path,
    workspace_root: &Path,
) -> Result<(), OrbitError> {
    init_workspace_at_root(
        global_root,
        InitOptions {
            global_only: true,
            ..Default::default()
        },
    )?;
    prepare_workspace_root_layout(workspace_root)?;
    if RuntimeConfig::load_layered(global_root, global_root)?.scoring_enabled {
        seed_scoreboard_templates(workspace_root)?;
    }
    Ok(())
}

/// Initialize the global `~/.orbit/` root. Always targets `~/.orbit/`
/// regardless of cwd, unless `--root` override is provided.
pub fn init_global(
    root_override: Option<&Path>,
    options: InitOptions,
) -> Result<InitResult, OrbitError> {
    let global_root = match root_override {
        Some(root) => root.to_path_buf(),
        None => crate::workspace_registry::global_orbit_dir()?,
    };
    init_workspace_at_root(
        &global_root,
        InitOptions {
            global_only: true,
            link_global_skills: true,
            ..options
        },
    )
}

pub fn init_workspace_from_root_override(
    root_override: Option<&Path>,
    options: InitOptions,
) -> Result<InitResult, OrbitError> {
    let cwd = std::env::current_dir().map_err(|e| OrbitError::Io(e.to_string()))?;
    let roots = OrbitRuntime::resolve_bootstrap_roots_for_cwd(&cwd, root_override)?;
    init_workspace_at_root(
        &roots.shared_root,
        InitOptions {
            global_root_override: Some(roots.global_root),
            ..options
        },
    )
}

pub fn init_workspace_at_root(
    orbit_root: &Path,
    options: InitOptions,
) -> Result<InitResult, OrbitError> {
    let init_target = resolve_init_target_from_root(orbit_root);
    let orbit_root = init_target.orbit_root.clone();

    if options.force {
        remove_path_if_exists(&orbit_root)?;
    }
    let layout = if options.global_only {
        prepare_global_root_layout(&orbit_root)?
    } else {
        prepare_workspace_root_layout(&orbit_root)?
    };
    let skills_root = if options.global_only {
        global_skills_dir(&orbit_root)
    } else {
        layout.skills_dir.clone()
    };

    let overwrite = options.force || options.refresh_defaults;
    let mut refreshed_skill_files = if options.global_only {
        seed_default_skills(&skills_root, &orbit_root, overwrite)?
    } else {
        0
    };
    let created_config = if options.global_only {
        let config_path = orbit_root.join("config.toml");
        seed_default_config(&config_path, options.role_settings.as_ref())?
    } else {
        false
    };

    let skill_ids = default_skill_ids();
    let mut created_skills_symlink = false;
    if options.global_only && options.link_global_skills {
        for skills_links_root in &init_target.skills_links_roots {
            created_skills_symlink |=
                ensure_skill_links(&skills_root, &skill_ids, skills_links_root, options.force)?;
        }
    }

    let (
        refreshed_default_activities,
        refreshed_default_jobs,
        refreshed_default_executors,
        refreshed_default_policies,
        scoring_enabled,
    ) = if options.global_only {
        let executor_store = global_executor_def_store(layout.executors_dir.clone());
        let policy_store = global_policy_def_store(layout.policies_dir.clone());
        let refreshed_default_executors =
            seed_default_executors(executor_store.as_ref(), overwrite)?;
        let refreshed_default_policies = seed_default_policies(policy_store.as_ref(), overwrite)?;
        let refreshed_default_activities =
            seed_default_activities(&layout.activities_dir, overwrite)?;
        let refreshed_default_jobs = seed_default_jobs(&layout.jobs_dir, overwrite)?;
        (
            refreshed_default_activities,
            refreshed_default_jobs,
            refreshed_default_executors,
            refreshed_default_policies,
            false,
        )
    } else {
        let global_root = options
            .global_root_override
            .clone()
            .map_or_else(resolve_global_root, Ok::<PathBuf, OrbitError>)?;
        let global_result = init_workspace_at_root(
            &global_root,
            InitOptions {
                refresh_defaults: options.refresh_defaults,
                global_only: true,
                link_global_skills: options.link_global_skills || options.refresh_defaults,
                role_settings: options.role_settings.clone(),
                ..Default::default()
            },
        )?;
        refreshed_skill_files = global_result.refreshed_skill_files;
        created_skills_symlink = global_result.created_skills_symlink;
        (
            global_result.refreshed_default_activities,
            global_result.refreshed_default_jobs,
            global_result.refreshed_default_executors,
            global_result.refreshed_default_policies,
            RuntimeConfig::load_layered(&global_root, &orbit_root)?.scoring_enabled,
        )
    };

    if scoring_enabled {
        seed_scoreboard_templates(&orbit_root)?;
    }
    if !options.global_only {
        friction_store::ensure_default_tag_taxonomy(&orbit_root.join("frictions"))?;
    }

    Ok(InitResult {
        refreshed_skill_files,
        created_skills_symlink,
        created_config,
        refreshed_default_activities,
        refreshed_default_jobs,
        refreshed_default_executors,
        refreshed_default_policies,
    })
}

pub fn seed_default_orbitignore(workspace_root: &Path) -> Result<bool, OrbitError> {
    let orbitignore_path = workspace_root.join(".orbitignore");
    if orbitignore_path.exists() {
        return Ok(false);
    }
    let template = crate::command::graph::default_orbitignore_template();
    atomic_write_text(&orbitignore_path, &template)
        .map_err(|error| OrbitError::Io(error.to_string()))?;
    Ok(true)
}

pub fn seed_design_conventions(repo_root: &Path, owner: &str) -> Result<bool, OrbitError> {
    let conventions_path = repo_root.join(DESIGN_DIR).join("CONVENTIONS.md");
    if conventions_path.exists() {
        return Ok(false);
    }
    let owner = normalize_owner(owner);
    let content = conventions_with_owner(&owner)?;
    atomic_write_text(&conventions_path, &content)
        .map_err(|error| OrbitError::Io(error.to_string()))?;
    Ok(true)
}

pub fn build_initial_graph(
    workspace_root: &Path,
    orbit_dir: &Path,
) -> Result<crate::command::graph::GraphBuildOutput, OrbitError> {
    crate::command::graph::build_graph(crate::command::graph::GraphBuildOptions {
        data_root: orbit_dir.to_path_buf(),
        repo_override: Some(workspace_root.to_path_buf()),
        ref_name: None,
        incremental: false,
    })
}

pub(crate) fn global_skills_dir(global_root: &Path) -> PathBuf {
    global_root.join("skills")
}

#[derive(Debug, Clone)]
struct InitTarget {
    orbit_root: PathBuf,
    skills_links_roots: Vec<PathBuf>,
}

fn resolve_init_target_from_root(orbit_root: &Path) -> InitTarget {
    let orbit_root = orbit_root.to_path_buf();
    let skills_links_base = crate::paths::home_dir()
        .or_else(|| find_git_repo_root(&orbit_root))
        .unwrap_or_else(|| {
            orbit_root
                .parent()
                .unwrap_or(orbit_root.as_path())
                .to_path_buf()
        });
    let skills_links_roots = skill_link_roots(&skills_links_base);

    InitTarget {
        orbit_root,
        skills_links_roots,
    }
}

pub(crate) fn skill_link_roots(base_root: &Path) -> Vec<PathBuf> {
    [".agents", ".claude"]
        .into_iter()
        .map(|dir| base_root.join(dir).join("skills"))
        .collect()
}

fn find_git_repo_root(start: &Path) -> Option<PathBuf> {
    crate::paths::find_git_repo_root(start)
}

fn conventions_with_owner(owner: &str) -> Result<String, OrbitError> {
    Ok(owner_regex()?
        .replacen(DESIGN_CONVENTIONS_TEMPLATE, 1, |captures: &Captures<'_>| {
            let matched = captures
                .get(0)
                .map(|value| value.as_str().trim_start())
                .unwrap_or_default();
            if matched.starts_with("owner:") {
                format!("owner: {owner}")
            } else {
                format!("**Owner:** {owner}")
            }
        })
        .to_string())
}

fn normalize_owner(owner: &str) -> String {
    let owner = owner.trim();
    if owner.is_empty() {
        "human".to_string()
    } else {
        owner.to_string()
    }
}

fn owner_regex() -> Result<&'static Regex, OrbitError> {
    static REGEX: OnceLock<Result<Regex, String>> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(r"(?m)^\s*(?:\*\*Owner:\*\*|owner:)\s*(.*?)\s*$")
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| OrbitError::Execution(format!("compile owner regex: {error}")))
}

fn seed_scoreboard_templates(orbit_root: &Path) -> Result<(), OrbitError> {
    let scoreboard_dir = orbit_layout_paths(orbit_root).scoreboard_dir;
    fs::create_dir_all(&scoreboard_dir).map_err(|e| OrbitError::Io(e.to_string()))?;

    let pr_path = scoreboard_dir.join("pr.json");
    if !pr_path.exists() {
        fs::write(&pr_path, "{}\n").map_err(|e| OrbitError::Io(e.to_string()))?;
    }

    let task_review_path = scoreboard_dir.join("task_review.json");
    if !task_review_path.exists() {
        fs::write(&task_review_path, "{}\n").map_err(|e| OrbitError::Io(e.to_string()))?;
    }

    Ok(())
}

fn prepare_workspace_root_layout(orbit_root: &Path) -> Result<WorkspacePaths, OrbitError> {
    fs::create_dir_all(orbit_root).map_err(|e| OrbitError::Io(e.to_string()))?;
    let layout = orbit_layout_paths(orbit_root);
    ensure_workspace_dirs(&layout)?;
    remove_workspace_seeded_default_skills(orbit_root, &layout)?;
    Ok(layout)
}

fn orbit_layout_paths(orbit_root: &Path) -> WorkspacePaths {
    let repo_root = orbit_root.parent().unwrap_or(orbit_root).to_path_buf();
    WorkspacePaths::new(
        repo_root,
        orbit_root.to_path_buf(),
        orbit_root.to_path_buf(),
    )
}

fn prepare_global_root_layout(orbit_root: &Path) -> Result<WorkspacePaths, OrbitError> {
    fs::create_dir_all(orbit_root).map_err(|e| OrbitError::Io(e.to_string()))?;
    let layout = orbit_layout_paths(orbit_root);
    ensure_global_dirs(&layout)?;
    Ok(layout)
}

fn ensure_workspace_dirs(paths: &WorkspacePaths) -> Result<(), OrbitError> {
    for dir in [
        &paths.resources_dir,
        &paths.state_dir,
        &paths.audit_dir,
        &paths.job_runs_dir,
        &paths.logs_dir,
        &paths.diagnostics_dir,
        &paths.scoreboard_dir,
        &paths.worktrees_dir,
        &paths.tasks_dir,
        &paths.knowledge_dir,
    ] {
        fs::create_dir_all(dir).map_err(|e| OrbitError::Io(e.to_string()))?;
    }
    Ok(())
}

fn remove_workspace_seeded_default_skills(
    orbit_root: &Path,
    paths: &WorkspacePaths,
) -> Result<(), OrbitError> {
    for skills_dir in [&paths.skills_dir, &orbit_root.join("skills")] {
        if !skills_dir.exists() {
            continue;
        }

        for skill_id in default_skill_ids() {
            let skill_dir = skills_dir.join(skill_id);
            let skill_file = skill_dir.join("SKILL.md");
            if is_default_skill_file_for_root(skill_id, &skill_file, orbit_root)? {
                remove_path_if_exists(&skill_dir)?;
            }
        }
        for skill_id in LEGACY_WORKSPACE_SEEDED_SKILL_IDS {
            remove_path_if_exists(&skills_dir.join(skill_id))?;
        }

        remove_empty_dir(skills_dir)?;
    }
    Ok(())
}

fn remove_empty_dir(dir: &Path) -> Result<(), OrbitError> {
    if !dir.is_dir() {
        return Ok(());
    }
    let mut entries = fs::read_dir(dir).map_err(|e| OrbitError::Io(e.to_string()))?;
    if entries.next().is_none() {
        fs::remove_dir(dir).map_err(|e| OrbitError::Io(e.to_string()))?;
    }
    Ok(())
}

fn ensure_global_dirs(paths: &WorkspacePaths) -> Result<(), OrbitError> {
    for dir in [
        &paths.resources_dir,
        &paths.activities_dir,
        &paths.jobs_dir,
        &paths.executors_dir,
        &paths.policies_dir,
        &global_skills_dir(&paths.orbit_dir),
    ] {
        fs::create_dir_all(dir).map_err(|e| OrbitError::Io(e.to_string()))?;
    }
    Ok(())
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

    if let Ok(metadata) = fs::symlink_metadata(skills_links_dir)
        && !metadata.file_type().is_dir()
    {
        if force {
            remove_path_if_exists(skills_links_dir)?;
        } else {
            return Err(OrbitError::InvalidInput(format!(
                "expected '{}' to be a directory for skill links; found non-directory path",
                skills_links_dir.display()
            )));
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

// --- Public link/unlink API ---

#[derive(Debug, Clone)]
pub struct LinkResult {
    pub linked_count: usize,
    pub roots: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct UnlinkResult {
    pub removed_count: usize,
    pub cleaned_dirs: Vec<PathBuf>,
}

/// Re-create skill symlinks in `~/.agents/skills/` and `~/.claude/skills/`.
pub fn link_skills(global_root: &Path) -> Result<LinkResult, OrbitError> {
    let init_target = resolve_init_target_from_root(global_root);
    let skills_root = global_skills_dir(&init_target.orbit_root);

    if !skills_root.exists() {
        return Err(OrbitError::InvalidInput(format!(
            "skills root does not exist: {}",
            skills_root.display()
        )));
    }

    let skill_ids = default_skill_ids();
    let mut linked_count = 0usize;
    let mut roots = Vec::new();

    for skills_links_root in &init_target.skills_links_roots {
        let changed = ensure_skill_links(&skills_root, &skill_ids, skills_links_root, false)?;
        if changed {
            linked_count += skill_ids.len();
        }
        roots.push(skills_links_root.clone());
    }

    Ok(LinkResult {
        linked_count,
        roots,
    })
}

/// Remove skill symlinks from `~/.agents/skills/` and `~/.claude/skills/`.
/// Only removes symlinks — regular files and directories are left intact.
pub fn unlink_skills(global_root: &Path) -> Result<UnlinkResult, OrbitError> {
    let init_target = resolve_init_target_from_root(global_root);
    let mut removed_count = 0usize;
    let mut cleaned_dirs = Vec::new();

    for skills_links_dir in &init_target.skills_links_roots {
        if !skills_links_dir.exists() {
            continue;
        }

        let entries = fs::read_dir(skills_links_dir).map_err(|e| OrbitError::Io(e.to_string()))?;

        for entry in entries {
            let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
            let meta =
                fs::symlink_metadata(entry.path()).map_err(|e| OrbitError::Io(e.to_string()))?;
            if meta.file_type().is_symlink() {
                fs::remove_file(entry.path()).map_err(|e| OrbitError::Io(e.to_string()))?;
                removed_count += 1;
            }
        }

        // Clean up empty skills dir, then empty parent (.agents/ or .claude/)
        if skills_links_dir.exists() && dir_is_empty(skills_links_dir)? {
            fs::remove_dir(skills_links_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
            cleaned_dirs.push(skills_links_dir.clone());

            if let Some(parent) = skills_links_dir.parent()
                && parent.exists()
                && dir_is_empty(parent)?
            {
                fs::remove_dir(parent).map_err(|e| OrbitError::Io(e.to_string()))?;
                cleaned_dirs.push(parent.to_path_buf());
            }
        }
    }

    Ok(UnlinkResult {
        removed_count,
        cleaned_dirs,
    })
}

fn dir_is_empty(path: &Path) -> Result<bool, OrbitError> {
    let mut entries = fs::read_dir(path).map_err(|e| OrbitError::Io(e.to_string()))?;
    Ok(entries.next().is_none())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use tempfile::tempdir;

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn seed_design_conventions_rewrites_owner_once() {
        let root = tempdir().expect("tempdir");
        let seeded = seed_design_conventions(root.path(), "codex").expect("seed conventions");
        assert!(seeded);
        let conventions = fs::read_to_string(root.path().join(DESIGN_DIR).join("CONVENTIONS.md"))
            .expect("read conventions");
        assert!(conventions.contains("owner: codex"));
        assert!(!conventions.contains("owner: daniel"));

        let second = seed_design_conventions(root.path(), "claude").expect("idempotent");
        assert!(!second);
        let conventions = fs::read_to_string(root.path().join(DESIGN_DIR).join("CONVENTIONS.md"))
            .expect("read conventions again");
        assert!(conventions.contains("owner: codex"));
        assert!(!conventions.contains("owner: claude"));
    }

    #[test]
    fn global_init_seeds_skills_and_home_level_links() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let home = tempdir().expect("home tempdir");
        let previous_home = std::env::var_os("HOME");
        unsafe {
            std::env::set_var("HOME", home.path());
        }

        let result = init_global(
            None,
            InitOptions {
                refresh_defaults: true,
                ..Default::default()
            },
        );

        restore_home(previous_home);

        let result = result.expect("init global");
        assert_eq!(result.refreshed_skill_files, default_skill_ids().len());
        assert!(result.created_skills_symlink);
        assert!(
            home.path()
                .join(".orbit")
                .join("skills")
                .join("orbit")
                .join("SKILL.md")
                .exists()
        );
        assert!(
            home.path()
                .join(".orbit")
                .join("skills")
                .join("orbit-debug-job-failure")
                .join("references")
                .join("common_failures.md")
                .exists()
        );
        assert!(
            !home
                .path()
                .join(".orbit")
                .join("resources")
                .join("skills")
                .join("orbit")
                .join("SKILL.md")
                .exists()
        );
        assert_skill_link_exists(home.path().join(".agents").join("skills").join("orbit"));
        assert_skill_link_exists(home.path().join(".claude").join("skills").join("orbit"));
    }

    #[test]
    fn workspace_init_leaves_repo_skills_unseeded() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let home = tempdir().expect("home tempdir");
        let workspace = tempdir().expect("workspace tempdir");
        let previous_home = std::env::var_os("HOME");
        unsafe {
            std::env::set_var("HOME", home.path());
        }

        let orbit_root = workspace.path().join(".orbit");
        seed_default_skills(
            &orbit_root.join("resources").join("skills"),
            &orbit_root,
            true,
        )
        .expect("seed legacy workspace resource skills");
        seed_default_skills(&orbit_root.join("skills"), &orbit_root, true)
            .expect("seed legacy workspace skills");
        let custom_skill = orbit_root.join("resources").join("skills").join("custom");
        fs::create_dir_all(&custom_skill).expect("create custom skill");
        fs::write(
            custom_skill.join("SKILL.md"),
            "# Custom\n\n## Purpose\n\nKeep me.\n",
        )
        .expect("write custom skill");
        let legacy_skill = orbit_root.join("resources").join("skills").join("orbit-pr");
        fs::create_dir_all(&legacy_skill).expect("create legacy skill");
        fs::write(
            legacy_skill.join("SKILL.md"),
            "---\nname: orbit-pr\n---\n\n# Orbit PR\n",
        )
        .expect("write legacy skill");

        let result = init_workspace_at_root(
            &orbit_root,
            InitOptions {
                refresh_defaults: true,
                global_root_override: Some(home.path().join(".orbit")),
                ..Default::default()
            },
        );

        restore_home(previous_home);

        let result = result.expect("init workspace");
        assert_eq!(result.refreshed_skill_files, default_skill_ids().len());
        assert!(result.created_skills_symlink);
        assert!(
            !orbit_root
                .join("resources")
                .join("skills")
                .join("orbit")
                .join("SKILL.md")
                .exists()
        );
        assert!(!orbit_root.join("skills").exists());
        assert!(orbit_root.join("state").join("logs").exists());
        assert!(custom_skill.join("SKILL.md").exists());
        assert!(!legacy_skill.exists());
        assert!(
            home.path()
                .join(".orbit")
                .join("skills")
                .join("orbit")
                .join("SKILL.md")
                .exists()
        );
        assert_skill_link_exists(home.path().join(".claude").join("skills").join("orbit"));
    }

    #[test]
    fn global_init_writes_role_settings_as_custom_crew_to_config_toml() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let home = tempdir().expect("home tempdir");
        let previous_home = std::env::var_os("HOME");
        unsafe {
            std::env::set_var("HOME", home.path());
        }

        let mut roles: BTreeMap<String, RawAgentRoleConfig> = BTreeMap::new();
        roles.insert(
            "reviewer".into(),
            RawAgentRoleConfig {
                provider: Some("claude".into()),
                backend: Some("cli".into()),
                model: Some("claude-opus-4-7".into()),
            },
        );
        roles.insert(
            "implementer".into(),
            RawAgentRoleConfig {
                provider: Some("codex".into()),
                backend: Some("cli".into()),
                model: Some("gpt-5.5".into()),
            },
        );
        roles.insert(
            "planner".into(),
            RawAgentRoleConfig {
                provider: Some("gemini".into()),
                backend: Some("http".into()),
                model: Some("gemini-3.1-pro".into()),
            },
        );

        let result = init_global(
            None,
            InitOptions {
                refresh_defaults: true,
                role_settings: Some(roles),
                ..Default::default()
            },
        );

        restore_home(previous_home);

        let result = result.expect("init global with role settings");
        assert!(result.created_config);

        let config_path = home.path().join(".orbit").join("config.toml");
        let contents = fs::read_to_string(&config_path).expect("read config");
        assert!(!contents.contains("[agent.reviewer]"));
        assert!(contents.contains("default_crew = \"custom\""));
        assert!(contents.contains("provider = \"codex\""));
        assert!(contents.contains("model = \"claude-opus-4-7\""));

        // Round-trips through toml: custom crew contains all three roles.
        let parsed: toml::Value = toml::from_str(&contents).expect("parse");
        let custom = parsed
            .get("crews")
            .and_then(|v| v.as_table())
            .and_then(|v| v.get("custom"))
            .and_then(|v| v.as_table())
            .expect("custom crew table");
        assert_eq!(custom.len(), 3);
        let reviewer = custom
            .get("reviewer")
            .and_then(|v| v.as_table())
            .expect("reviewer table");
        assert_eq!(
            reviewer.get("provider").and_then(|v| v.as_str()),
            Some("claude")
        );
        let planner = custom
            .get("planner")
            .and_then(|v| v.as_table())
            .expect("planner table");
        assert_eq!(
            planner.get("model").and_then(|v| v.as_str()),
            Some("gemini-3.1-pro")
        );
    }

    #[test]
    fn global_init_with_existing_config_does_not_overwrite_role_settings() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let home = tempdir().expect("home tempdir");
        let previous_home = std::env::var_os("HOME");
        unsafe {
            std::env::set_var("HOME", home.path());
        }

        // Pre-seed config.toml with user content.
        let orbit_root = home.path().join(".orbit");
        fs::create_dir_all(&orbit_root).expect("mkdir .orbit");
        let config_path = orbit_root.join("config.toml");
        let user_content = "# pre-existing user config\n";
        fs::write(&config_path, user_content).expect("preseed");

        let mut roles: BTreeMap<String, RawAgentRoleConfig> = BTreeMap::new();
        roles.insert(
            "reviewer".into(),
            RawAgentRoleConfig {
                provider: Some("claude".into()),
                backend: Some("cli".into()),
                model: None,
            },
        );

        let result = init_global(
            None,
            InitOptions {
                refresh_defaults: true,
                role_settings: Some(roles),
                ..Default::default()
            },
        );

        restore_home(previous_home);

        let result = result.expect("init global");
        assert!(!result.created_config);
        let final_contents = fs::read_to_string(&config_path).expect("read config");
        assert_eq!(final_contents, user_content);
    }

    #[test]
    fn global_init_without_role_settings_writes_clean_template() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let home = tempdir().expect("home tempdir");
        let previous_home = std::env::var_os("HOME");
        unsafe {
            std::env::set_var("HOME", home.path());
        }

        let result = init_global(
            None,
            InitOptions {
                refresh_defaults: true,
                role_settings: None,
                ..Default::default()
            },
        );

        restore_home(previous_home);

        let result = result.expect("init global");
        assert!(result.created_config);
        let config_path = home.path().join(".orbit").join("config.toml");
        let contents = fs::read_to_string(&config_path).expect("read config");
        for line in contents.lines() {
            assert!(
                !line.trim_start().starts_with("[agent."),
                "unexpected uncommented agent section: {line}",
            );
        }
        assert!(contents.contains("[crews.opus-codex]"));
        assert!(contents.contains("[crews.all-claude]"));
        assert!(contents.contains("default_crew = \"opus-codex\""));
    }

    fn assert_skill_link_exists(path: PathBuf) {
        let metadata = fs::symlink_metadata(&path).expect("link metadata");
        assert!(
            metadata.file_type().is_symlink(),
            "expected {} to be a symlink",
            path.display()
        );
        assert!(path.join("SKILL.md").exists());
    }

    fn restore_home(previous_home: Option<std::ffi::OsString>) {
        match previous_home {
            Some(value) => unsafe {
                std::env::set_var("HOME", value);
            },
            None => unsafe {
                std::env::remove_var("HOME");
            },
        }
    }
}
