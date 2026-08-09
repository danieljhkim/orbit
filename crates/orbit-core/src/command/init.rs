use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use orbit_common::types::{OrbitError, WorkspacePaths};
use orbit_store::{friction_store, global_executor_def_store, global_policy_def_store};

use crate::OrbitRuntime;
use crate::auto_tasks::seed_default_auto_tasks;
use crate::command::activity::seed_default_activities;
use crate::command::executor::seed_default_executors;
use crate::command::job::seed_default_jobs;
use crate::command::policy::seed_default_policies;
use crate::command::routine::seed_default_routines;
use crate::command::skill::{
    default_skill_ids, is_default_skill_file_for_root, seed_default_skills,
};
use orbit_common::utility::fs::{create_dir_symlink, remove_path_if_exists};

use crate::config::{
    RawCrewAssignment, RuntimeConfig,
    agent_detect::{DetectedAgents, RealAgentEnvProbe, detect},
    seed_default_config,
};
use crate::runtime::{is_global_orbit_root, resolve_global_root};

const LEGACY_WORKSPACE_SEEDED_SKILL_IDS: [&str; 2] = ["orbit-approve-task", "orbit-pr"];

#[derive(Debug, Clone)]
pub struct InitResult {
    pub refreshed_skill_files: usize,
    pub created_skills_symlink: bool,
    pub created_config: bool,
    pub refreshed_default_activities: usize,
    pub retired_default_activities: usize,
    pub refreshed_default_jobs: usize,
    pub retired_default_jobs: usize,
    pub managed_asset_warnings: Vec<String>,
    pub refreshed_default_executors: usize,
    pub refreshed_default_policies: usize,
    pub refreshed_default_routines: usize,
    pub seeded_default_auto_tasks: usize,
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
    /// Host id to pin into newly seeded workspace routines. Higher-level
    /// composition owns host identity and supplies this value explicitly.
    pub routine_host_id: Option<String>,
    /// When true, create/update user-level skill symlinks for global skills.
    pub link_global_skills: bool,
    /// Crew settings collected by the init prompt. The `custom` assignment is
    /// embedded as the flat `[crews.custom]` table. `None` and
    /// an empty map both mean "use
    /// the default crew template". Ignored when config.toml already exists
    /// — init remains idempotent.
    pub crew_settings: Option<BTreeMap<String, RawCrewAssignment>>,
    /// Agent availability snapshot used to freeze agent-dependent config at
    /// init time. When omitted, init probes the real host environment.
    pub detected: Option<DetectedAgents>,
}

impl OrbitRuntime {
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
        None => resolve_global_root()?,
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
        let detected = options
            .detected
            .clone()
            .unwrap_or_else(|| detect(&RealAgentEnvProbe));
        seed_default_config(&config_path, &detected, options.crew_settings.as_ref())?
    } else {
        false
    };

    let skill_ids = default_skill_ids();
    let mut created_skills_symlink = false;
    // Home-scoped skill link directories (`~/.agents/skills`, `~/.claude/skills`)
    // are only ever touched for the true global Orbit root. A non-global root
    // (a validation root, an alternate `--root`, a workspace root) must leave
    // them exactly as found — no removal, no re-creation, no replacement.
    if options.global_only && options.link_global_skills && is_global_orbit_root(&orbit_root) {
        for skills_links_root in &init_target.skills_links_roots {
            created_skills_symlink |=
                ensure_skill_links(&skills_root, &skill_ids, skills_links_root, options.force)?;
        }
    }

    let mut refreshed_default_routines = 0usize;
    let mut seeded_default_auto_tasks = 0usize;
    let (
        refreshed_default_activities,
        retired_default_activities,
        refreshed_default_jobs,
        retired_default_jobs,
        managed_asset_warnings,
        refreshed_default_executors,
        refreshed_default_policies,
        scoring_enabled,
    ) = if options.global_only {
        let executor_store = global_executor_def_store(layout.executors_dir.clone());
        let policy_store = global_policy_def_store(layout.policies_dir.clone());
        let refreshed_default_executors =
            seed_default_executors(executor_store.as_ref(), overwrite)?;
        let refreshed_default_policies = seed_default_policies(policy_store.as_ref(), overwrite)?;
        let activity_reconciliation = seed_default_activities(&layout.activities_dir, overwrite)?;
        let job_reconciliation = seed_default_jobs(&layout.jobs_dir, overwrite)?;
        let mut managed_asset_warnings = activity_reconciliation.warnings;
        managed_asset_warnings.extend(job_reconciliation.warnings);
        (
            activity_reconciliation.refreshed,
            activity_reconciliation.retired,
            job_reconciliation.refreshed,
            job_reconciliation.retired,
            managed_asset_warnings,
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
                crew_settings: options.crew_settings.clone(),
                detected: options.detected.clone(),
                ..Default::default()
            },
        )?;
        refreshed_skill_files = global_result.refreshed_skill_files;
        created_skills_symlink = global_result.created_skills_symlink;
        // Routines are workspace-authored (`.orbit/routines/`, no global
        // directory), so defaults seed here rather than in the global branch.
        // Host identity is owned by higher-level composition and injected;
        // Core never opens host.toml or falls back to an OS hostname.
        if let Some(host_id) = options.routine_host_id.as_deref() {
            refreshed_default_routines = seed_default_routines(
                &orbit_root.join("routines"),
                host_id,
                workspace_slug_from_orbit_root(&orbit_root).as_deref(),
                // Routine definitions become workspace-authored after
                // seeding. Refresh global defaults without overwriting
                // cadence, host pins, policy, or enabled choices here;
                // destructive `force` already recreated the root.
                options.force,
            )?;
        }
        // Auto-task definitions are workspace-authored after seeding. Never
        // refresh an existing file: `workspace init --force` reconciles
        // registration and must not overwrite an operator's definition.
        seeded_default_auto_tasks = seed_default_auto_tasks(&orbit_root)?;
        (
            global_result.refreshed_default_activities,
            global_result.retired_default_activities,
            global_result.refreshed_default_jobs,
            global_result.retired_default_jobs,
            global_result.managed_asset_warnings,
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
        retired_default_activities,
        refreshed_default_jobs,
        retired_default_jobs,
        managed_asset_warnings,
        refreshed_default_executors,
        refreshed_default_policies,
        refreshed_default_routines,
        seeded_default_auto_tasks,
    })
}

/// Derive the routine-name suffix for seeded default routines from the
/// workspace directory containing `.orbit/`.
fn workspace_slug_from_orbit_root(orbit_root: &Path) -> Option<String> {
    orbit_root
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
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
    fn fresh_workspace_init_seeds_disabled_worktree_gc_routine() {
        let temp = tempdir().expect("tempdir");
        let global_root = temp.path().join("global");
        let orbit_root = temp.path().join("repo/.orbit");
        let result = init_workspace_at_root(
            &orbit_root,
            InitOptions {
                global_root_override: Some(global_root.clone()),
                refresh_defaults: true,
                routine_host_id: Some("host-a".to_string()),
                ..Default::default()
            },
        )
        .expect("initialize fresh workspace");

        assert_eq!(
            result.refreshed_default_routines,
            super::super::routine::DEFAULT_ROUTINE_FILES.len()
        );
        let yaml = fs::read_to_string(orbit_root.join("routines/worktree_gc.yaml"))
            .expect("read seeded worktree GC routine");
        assert!(!yaml.contains("__ORBIT_"));
        let routine = orbit_common::types::parse_routine_yaml(&yaml)
            .expect("seeded worktree GC routine parses");
        assert!(!routine.enabled);
        assert_eq!(routine.hosts, vec!["host-a".to_string()]);
        assert_eq!(
            routine.target,
            orbit_common::types::RoutineTarget::Job("worktree_gc_pipeline".to_string())
        );
        assert_eq!(
            routine.policy.overlap,
            orbit_common::types::OverlapPolicy::Forbid
        );

        let routine_path = orbit_root.join("routines/worktree_gc.yaml");
        fs::write(&routine_path, "operator edited").expect("hand edit routine");
        init_workspace_at_root(
            &orbit_root,
            InitOptions {
                global_root_override: Some(global_root.clone()),
                refresh_defaults: true,
                routine_host_id: Some("host-a".to_string()),
                ..Default::default()
            },
        )
        .expect("plain re-init");
        assert_eq!(
            fs::read_to_string(&routine_path).expect("read preserved routine"),
            "operator edited",
            "plain re-init preserves a hand-edited routine"
        );

        init_workspace_at_root(
            &orbit_root,
            InitOptions {
                global_root_override: Some(global_root),
                force: true,
                refresh_defaults: true,
                routine_host_id: Some("host-b".to_string()),
                ..Default::default()
            },
        )
        .expect("forced re-init");
        let forced =
            fs::read_to_string(&routine_path).expect("read force-overwritten worktree GC routine");
        assert!(!forced.contains("operator edited"));
        let forced = orbit_common::types::parse_routine_yaml(&forced)
            .expect("force-overwritten routine parses");
        assert_eq!(forced.hosts, vec!["host-b".to_string()]);
        assert!(!forced.enabled);
    }

    #[test]
    fn workspace_init_seeds_inert_defaults_without_clobbering_edits() {
        let temp = tempdir().expect("tempdir");
        let global_root = temp.path().join("global");
        let orbit_root = temp.path().join("repo/.orbit");
        let options = InitOptions {
            global_root_override: Some(global_root),
            detected: Some(DetectedAgents::default()),
            ..Default::default()
        };

        let initial = init_workspace_at_root(&orbit_root, options.clone())
            .expect("initialize fresh workspace");
        assert_eq!(initial.seeded_default_auto_tasks, 2);
        let friction_path = orbit_root.join("auto_tasks/friction-curation.yaml");
        let qa_path = orbit_root.join("auto_tasks/qa-sweep.yaml");
        let friction = fs::read_to_string(&friction_path).expect("read seeded friction definition");
        let friction_definition = orbit_common::types::parse_auto_task_yaml(&friction)
            .expect("seeded friction definition parses through loader schema");
        assert!(!friction_definition.enabled);
        assert_eq!(friction_definition.template.crew.as_deref(), Some("luna"));
        assert!(matches!(
            friction_definition.dedupe,
            orbit_common::types::DedupePolicy::SkipIfOpen
        ));
        let qa = fs::read_to_string(&qa_path).expect("read seeded QA definition");
        let qa_definition = orbit_common::types::parse_auto_task_yaml(&qa)
            .expect("seeded QA definition parses through loader schema");
        assert!(!qa_definition.enabled);
        assert_eq!(qa_definition.template.crew.as_deref(), Some("sonnet"));
        assert!(matches!(
            qa_definition.dedupe,
            orbit_common::types::DedupePolicy::SkipIfOpen
        ));
        assert!(!orbit_root.join("state/auto-tasks.json").exists());
        let loaded = crate::auto_tasks::collect_auto_tasks(&orbit_root);
        assert!(
            loaded.errors.is_empty(),
            "seeded definition must load cleanly"
        );
        assert_eq!(loaded.definitions.len(), 2);
        assert!(
            loaded
                .definitions
                .iter()
                .any(|loaded| loaded.definition.name == "friction-curation")
        );
        assert!(
            loaded
                .definitions
                .iter()
                .any(|loaded| loaded.definition.name == "qa-sweep")
        );

        let authored_friction = "operator-authored friction definition\n";
        let authored_qa = "operator-authored QA definition\n";
        fs::write(&friction_path, authored_friction).expect("write friction edit");
        fs::write(&qa_path, authored_qa).expect("write QA edit");
        let repeated =
            init_workspace_at_root(&orbit_root, options).expect("reinitialize workspace");
        assert_eq!(repeated.seeded_default_auto_tasks, 0);
        assert_eq!(
            fs::read_to_string(friction_path).expect("read preserved friction definition"),
            authored_friction
        );
        assert_eq!(
            fs::read_to_string(qa_path).expect("read preserved QA definition"),
            authored_qa
        );
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
                detected: Some(DetectedAgents::default()),
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
                .join("orbit-workflow")
                .join("references")
                .join("debug-job-failure.md")
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
                detected: Some(DetectedAgents::default()),
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
    fn global_init_writes_crew_settings_as_custom_crew_to_config_toml() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let home = tempdir().expect("home tempdir");
        let previous_home = std::env::var_os("HOME");
        unsafe {
            std::env::set_var("HOME", home.path());
        }

        let settings = BTreeMap::from([(
            "custom".into(),
            RawCrewAssignment {
                provider: Some("codex".into()),
                backend: Some("cli".into()),
                model: Some(orbit_common::test_fixtures::TEST_CODEX_MODEL.into()),
            },
        )]);

        let result = init_global(
            None,
            InitOptions {
                refresh_defaults: true,
                crew_settings: Some(settings),
                detected: Some(DetectedAgents::default()),
                ..Default::default()
            },
        );

        restore_home(previous_home);

        let result = result.expect("init global with crew settings");
        assert!(result.created_config);

        let config_path = home.path().join(".orbit").join("config.toml");
        let contents = fs::read_to_string(&config_path).expect("read config");
        assert!(!contents.contains("[agent.reviewer]"));
        assert!(contents.contains("default_crew = \"custom\""));
        assert!(contents.contains("provider = \"codex\""));
        assert!(contents.contains(&format!(
            "model = \"{}\"",
            orbit_common::test_fixtures::TEST_CODEX_MODEL
        )));

        // Round-trips through toml: custom crew is one flat assignment.
        let parsed: toml::Value = toml::from_str(&contents).expect("parse");
        let custom = parsed
            .get("crews")
            .and_then(|v| v.as_table())
            .and_then(|v| v.get("custom"))
            .and_then(|v| v.as_table())
            .expect("custom crew table");
        assert_eq!(custom.len(), 3);
        assert_eq!(
            custom.get("provider").and_then(|v| v.as_str()),
            Some("codex")
        );
        assert_eq!(
            custom.get("model").and_then(|v| v.as_str()),
            Some(orbit_common::test_fixtures::TEST_CODEX_MODEL)
        );
    }

    #[test]
    fn global_init_with_existing_config_does_not_overwrite_crew_settings() {
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

        let settings = BTreeMap::from([(
            "custom".into(),
            RawCrewAssignment {
                provider: Some("claude".into()),
                backend: Some("cli".into()),
                model: None,
            },
        )]);

        let result = init_global(
            None,
            InitOptions {
                refresh_defaults: true,
                crew_settings: Some(settings),
                detected: Some(DetectedAgents::default()),
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
    fn global_init_without_crew_settings_writes_clean_template() {
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
                crew_settings: None,
                detected: Some(DetectedAgents::default()),
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
        assert!(!contents.contains("[crews."));
        assert!(!contents.contains("default_crew"));
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
