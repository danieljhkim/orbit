//! Routine discovery [ORB-10021]: enumerate the global workspace registry,
//! visit every registered, active workspace whose versioned config declares
//! `[routines] role = "source"` (ADR-0205), and load `.orbit/routines/*.yaml`
//! from each — fail-closed per file. An invalid definition becomes a load
//! error and that routine is treated as absent; it never fires with defaults.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use orbit_common::types::{
    OrbitError, RoutineDefinition, Workspace, WorkspaceStatus, parse_routine_yaml,
};

use crate::OrbitRuntime;
use crate::workspace_registry;

use super::due::parse_cron;

/// Directory under a source workspace's `.orbit/` holding routine YAML files.
pub const ROUTINES_DIR: &str = "routines";

/// One successfully loaded, fully validated routine.
#[derive(Debug, Clone)]
pub struct LoadedRoutine {
    /// The parsed definition.
    pub definition: RoutineDefinition,
    /// Registry name of the source workspace.
    pub source_workspace: String,
    /// The source workspace's `.orbit` directory (dispatch root).
    pub source_orbit_dir: PathBuf,
    /// Path of the YAML file the definition came from.
    pub path: PathBuf,
}

/// One fail-closed load failure, kept for reporting: the routine (or source)
/// it names is treated as absent this pass.
#[derive(Debug, Clone)]
pub struct RoutineLoadError {
    /// Registry name of the source workspace involved.
    pub source_workspace: String,
    /// File that failed, when the failure is file-scoped.
    pub path: Option<PathBuf>,
    /// Human-readable reason.
    pub message: String,
}

/// Result of one discovery pass across all routine sources.
#[derive(Debug, Default)]
pub struct RoutineCollection {
    /// Valid routines, in stable (workspace, filename) order.
    pub routines: Vec<LoadedRoutine>,
    /// Everything that failed fail-closed.
    pub errors: Vec<RoutineLoadError>,
}

/// Registered workspaces with their runtimes, ready for routine discovery
/// and dispatch, plus the workspaces that failed to open (reported loudly —
/// registry hygiene must not silently shrink the source set).
#[derive(Default)]
pub struct DiscoveredWorkspaces {
    /// Active, openable workspaces (source or not) with their runtimes.
    pub entries: Vec<(Workspace, OrbitRuntime)>,
    /// Registered workspaces that could not be opened.
    pub errors: Vec<RoutineLoadError>,
}

/// Enumerate the global registry (validating and persisting hygiene fixes,
/// like `ship-sweep` does) and build one runtime per active workspace.
pub fn discover_workspaces(global_root: &Path) -> Result<DiscoveredWorkspaces, OrbitError> {
    let registry_path = workspace_registry::registry_path_for(global_root);
    let mut registry = workspace_registry::load_registry_from(&registry_path)?;
    workspace_registry::validate_workspaces(&mut registry);
    workspace_registry::save_registry_to(&registry, &registry_path)?;

    let mut discovered = DiscoveredWorkspaces::default();
    for workspace in registry.workspaces {
        if workspace.status != WorkspaceStatus::Active || !workspace.orbit_dir.exists() {
            continue;
        }
        match OrbitRuntime::from_roots(global_root, &workspace.orbit_dir) {
            Ok(runtime) => discovered.entries.push((workspace, runtime)),
            Err(error) => discovered.errors.push(RoutineLoadError {
                source_workspace: workspace.name.clone(),
                path: Some(workspace.orbit_dir.clone()),
                message: format!("failed to open workspace runtime: {error}"),
            }),
        }
    }
    Ok(discovered)
}

/// Load routines from every source workspace among `workspaces` (the same
/// runtimes are later used for dispatch). Cross-source name collisions are
/// load-time errors: every colliding definition is dropped.
pub fn collect_routines(workspaces: &[(Workspace, OrbitRuntime)]) -> RoutineCollection {
    let mut collection = RoutineCollection::default();

    for (workspace, runtime) in workspaces {
        if !runtime.routines_source() {
            continue;
        }
        load_source_workspace(workspace, runtime, &mut collection);
    }

    drop_name_collisions(&mut collection);
    collection
}

fn load_source_workspace(
    workspace: &Workspace,
    runtime: &OrbitRuntime,
    collection: &mut RoutineCollection,
) {
    let routines_dir = workspace.orbit_dir.join(ROUTINES_DIR);
    if !routines_dir.is_dir() {
        // A source with no routines directory is simply an empty source.
        return;
    }

    let mut paths: Vec<PathBuf> = match std::fs::read_dir(&routines_dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| {
                path.extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| {
                        ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml")
                    })
            })
            .collect(),
        Err(error) => {
            collection.errors.push(RoutineLoadError {
                source_workspace: workspace.name.clone(),
                path: Some(routines_dir),
                message: format!("failed to list routines directory: {error}"),
            });
            return;
        }
    };
    paths.sort();

    for path in paths {
        match load_routine_file(&path, workspace, runtime) {
            Ok(routine) => collection.routines.push(routine),
            Err(message) => collection.errors.push(RoutineLoadError {
                source_workspace: workspace.name.clone(),
                path: Some(path),
                message,
            }),
        }
    }
}

fn load_routine_file(
    path: &Path,
    workspace: &Workspace,
    runtime: &OrbitRuntime,
) -> Result<LoadedRoutine, String> {
    let raw = std::fs::read_to_string(path).map_err(|error| format!("read failed: {error}"))?;
    let definition = parse_routine_yaml(&raw).map_err(|error| error.to_string())?;

    // Load-time cron validation: a routine with an unparsable trigger never
    // reaches the due computation.
    parse_cron(&definition.trigger.cron).map_err(|error| error.to_string())?;

    // Load-time target resolution through the source workspace's catalog,
    // like `target:` steps in JobV2: an unresolvable target is a load error,
    // not a fire-time surprise (ADR-0206).
    let job_name = definition.target.job_name();
    runtime.load_v2_job_asset_by_name(job_name).map_err(|_| {
        format!(
            "target 'job:{job_name}' does not resolve in workspace '{}': \
             no such job in its catalog",
            workspace.name
        )
    })?;

    Ok(LoadedRoutine {
        definition,
        source_workspace: workspace.name.clone(),
        source_orbit_dir: workspace.orbit_dir.clone(),
        path: path.to_path_buf(),
    })
}

/// Names must be unique across all routine sources on a host; a collision is
/// a load-time error and every colliding definition is treated as absent
/// (fail-closed — firing an arbitrary winner would make behavior depend on
/// registry iteration order).
fn drop_name_collisions(collection: &mut RoutineCollection) {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for routine in &collection.routines {
        *counts.entry(routine.definition.name.clone()).or_default() += 1;
    }
    let colliding: Vec<String> = counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(name, _)| name)
        .collect();
    if colliding.is_empty() {
        return;
    }
    let mut kept = Vec::with_capacity(collection.routines.len());
    for routine in collection.routines.drain(..) {
        if colliding.contains(&routine.definition.name) {
            collection.errors.push(RoutineLoadError {
                source_workspace: routine.source_workspace.clone(),
                path: Some(routine.path.clone()),
                message: format!(
                    "routine name '{}' is defined by more than one source; \
                     names must be unique across all routine sources",
                    routine.definition.name
                ),
            });
        } else {
            kept.push(routine);
        }
    }
    collection.routines = kept;
}
